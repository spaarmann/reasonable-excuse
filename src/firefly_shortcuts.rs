use std::{net::SocketAddr, sync::Arc};

use axum::{extract::ConnectInfo, http::StatusCode, Extension, Json, Router};
use miette::{Context, IntoDiagnostic};
use reqwest::{Client, Method, RequestBuilder, Url};

#[derive(knuffel::Decode, Debug, serde::Serialize)]
struct Shortcut {
    shortcut_id: u64,
    #[knuffel(argument)]
    shortcut_name: String,
    #[knuffel(property(name = "icon"))]
    shortcut_icon: String,

    #[knuffel(child, unwrap(argument))]
    name: String,
    #[knuffel(child, unwrap(argument))]
    source: String,
    #[knuffel(child, unwrap(argument))]
    destination: String,
    #[knuffel(child, unwrap(argument))]
    amount: Option<f32>,
    #[knuffel(child, unwrap(argument))]
    budget: Option<String>,
    #[knuffel(child, unwrap(argument))]
    category: Option<String>,
}

#[derive(knuffel::Decode, Debug)]
pub struct Config {
    #[knuffel(child, unwrap(argument))]
    route: String,
    #[knuffel(child, unwrap(argument, str))]
    firefly_url: Url,
    #[knuffel(child, unwrap(argument))]
    pat_file: String,
    #[knuffel(children(name = "shortcut"))]
    shortcuts: Vec<Shortcut>,
}

/// A Firefly Personal Access Token.
#[derive(Clone, Debug)]
struct Pat(String);

pub fn setup(mut config: Config, app: Router) -> miette::Result<Router> {
    // Generate IDs for all of the shortcuts.
    for (i, shortcut) in config.shortcuts.iter_mut().enumerate() {
        shortcut.shortcut_id = i as u64;
    }

    let config = Arc::new(config);

    let client = Client::builder()
        .user_agent(concat!("reasonable-excuse/", env!("CARGO_PKG_VERSION")))
        .build()
        .into_diagnostic()
        .context("create reqwest Client")?;

    let pat = std::fs::read_to_string(&config.pat_file)
        .into_diagnostic()
        .with_context(|| format!("read firefly PAT from file: {}", config.pat_file))?;
    let pat = pat.trim_end().to_string();
    let pat = Arc::new(Pat(pat));

    let base = &config.route;
    Ok(app
        .route(
            &format!("{base}/shortcuts"),
            axum::routing::get(get_shortcuts),
        )
        .route(
            &format!("{base}/add-transaction"),
            axum::routing::post(add_transaction),
        )
        .layer(Extension(config))
        .layer(Extension(pat))
        .layer(Extension(client)))
}

#[tracing::instrument]
async fn get_shortcuts(Extension(config): Extension<Arc<Config>>) -> Result<String, StatusCode> {
    tracing::info!("get_shortcuts request");

    let json = serde_json::to_string_pretty(&config.shortcuts).map_err(|e| {
        tracing::error!("Failed to serialize shortcuts: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(json)
}

#[derive(Debug, serde::Deserialize)]
struct AddTransactionRequest {
    shortcut_id: u64,
    amount_override: Option<f32>,
}

#[tracing::instrument(skip(config, client, pat))]
async fn add_transaction(
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    Extension(config): Extension<Arc<Config>>,
    Extension(client): Extension<Client>,
    Extension(pat): Extension<Arc<Pat>>,
    Json(req): Json<AddTransactionRequest>,
) -> Result<String, StatusCode> {
    tracing::info!("add_transaction request");

    // Find shortcut with the given ID.
    let Some(shortcut) = config
        .shortcuts
        .iter()
        .find(|s| s.shortcut_id == req.shortcut_id)
    else {
        tracing::error!("Invalid shortcut ID");
        return Err(StatusCode::BAD_REQUEST);
    };

    // Resolve budget name to budget ID, if any.
    let budget_id = resolve_budget(shortcut.budget.as_ref(), &config, &client, &pat)
        .await
        .map_err(|e| {
            tracing::error!("Could not resolve budget ID: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Build and send the transaction to the Firefly server.
    let firefly_request =
        make_store_transaction_request(shortcut, req.amount_override, budget_id.as_ref()).map_err(
            |e| {
                tracing::error!("Could not make store transaction request: {e:?}");
                StatusCode::BAD_REQUEST
            },
        )?;
    let response = firefly_req(&config, &client, &pat, Method::POST, "/v1/transactions")
        .json(&firefly_request)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to send store transaction request: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let status_error = response.error_for_status_ref().err();

    let response_text = response.text().await.map_err(|e| {
        tracing::error!("Failed to read response text: {e:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    match status_error {
        Some(e) => {
            tracing::error!("Got API error: {e:?}, response: {response_text}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
        None => Ok(response_text),
    }
}

#[derive(Debug, serde::Deserialize)]
struct FireflyBudget {
    id: String,
    attributes: FireflyBudgetAttribs,
}

#[derive(Debug, serde::Deserialize)]
struct FireflyBudgetAttribs {
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct FireflyBudgetList {
    data: Vec<FireflyBudget>,
}

async fn resolve_budget(
    budget: Option<&String>,
    config: &Config,
    client: &Client,
    pat: &Pat,
) -> miette::Result<Option<String>> {
    let Some(budget_name) = budget else {
        return Ok(None);
    };

    let budgets = firefly_req(config, client, pat, Method::GET, "/v1/budgets")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .into_diagnostic()
        .context("fetching budgets")?
        .json::<FireflyBudgetList>()
        .await
        .into_diagnostic()
        .context("parsing budgets")?;

    for budget in budgets.data {
        if &budget.attributes.name == budget_name {
            return Ok(Some(budget.id));
        }
    }

    miette::bail!("Could not find budget with name {budget_name}");
}

#[derive(Debug, serde::Serialize)]
struct FireflyStoreTransactionRequest {
    error_if_duplicate_hash: bool,
    apply_rules: bool,
    fire_webhooks: bool,
    transactions: Vec<FireflyStoreTransactionSplit>,
}

#[derive(Debug, serde::Serialize)]
struct FireflyStoreTransactionSplit {
    #[serde(rename = "type")]
    transaction_type: String,
    date: String,
    amount: String,
    description: String,
    budget_id: Option<String>,
    category_name: Option<String>,
    source_name: String,
    destination_name: String,
}

fn make_store_transaction_request(
    shortcut: &Shortcut,
    amount_override: Option<f32>,
    budget_id: Option<&String>,
) -> miette::Result<FireflyStoreTransactionRequest> {
    let Some(amount) = amount_override.or(shortcut.amount) else {
        miette::bail!("Must have at least one of shortcut.amount or amount_override");
    };

    // 2018-09-17T12:46:47+01:00
    let date = format!("{}", chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z"));

    Ok(FireflyStoreTransactionRequest {
        error_if_duplicate_hash: true,
        apply_rules: true,
        fire_webhooks: true,
        transactions: vec![FireflyStoreTransactionSplit {
            transaction_type: "withdrawal".to_string(),
            date: date,
            amount: amount.to_string(),
            description: shortcut.name.clone(),
            budget_id: budget_id.cloned(),
            category_name: shortcut.category.clone(),
            source_name: shortcut.source.clone(),
            destination_name: shortcut.destination.clone(),
        }],
    })
}

fn firefly_req(
    config: &Config,
    client: &Client,
    pat: &Pat,
    method: Method,
    endpoint: &str,
) -> RequestBuilder {
    client
        .request(method, format!("{}api{}", config.firefly_url, endpoint))
        .bearer_auth(&pat.0)
        .header("accept", "application/vnd.api+json")
}

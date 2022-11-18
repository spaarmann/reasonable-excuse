use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, Query},
    http::StatusCode,
    Extension, Router,
};
use miette::{Context, IntoDiagnostic};
use regex::Regex;
use reqwest::{Client, Url};

#[derive(knuffel::Decode, Debug)]
pub struct Config {
    #[knuffel(child, unwrap(argument))]
    route: String,
    #[knuffel(child, unwrap(argument))]
    base_url: String,
    #[knuffel(child, unwrap(argument))]
    pass_param: String,
    #[knuffel(child, unwrap(argument))]
    filter: String,
}

pub fn setup(config: Config, app: Router) -> miette::Result<Router> {
    let config = Arc::new(config);
    let client = Client::builder()
        .user_agent(concat!("reasonable-excuse/", env!("CARGO_PKG_VERSION")))
        .build()
        .into_diagnostic()
        .wrap_err("Failed to create reqwest Client")?;

    let filter_regex = Regex::new(&config.filter)
        .into_diagnostic()
        .wrap_err("Failed to create filter regex")?;

    Ok(app
        .route(&config.route, axum::routing::get(get))
        .layer(Extension(config))
        .layer(Extension(filter_regex))
        .layer(Extension(client)))
}

#[tracing::instrument(skip(client))]
async fn get(
    Query(params): Query<HashMap<String, String>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    Extension(config): Extension<Arc<Config>>,
    Extension(filter): Extension<Regex>,
    Extension(client): Extension<Client>,
) -> Result<String, StatusCode> {
    tracing::info!("Calendar request");

    let param = params.get(&config.pass_param).ok_or_else(|| {
        tracing::warn!("Bad calendar request, no {} query param", config.pass_param);
        StatusCode::BAD_REQUEST
    })?;

    let url =
        Url::parse_with_params(&config.base_url, &[(&config.pass_param, param)]).map_err(|e| {
            tracing::error!("Failed to construct calendar request URL: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let response = client.get(url).send().await.map_err(|e| {
        tracing::error!("Failed to get base calendar: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = response.error_for_status().map_err(|e| {
        tracing::error!("Failed to get base calendar: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = response.text().await.map_err(|e| {
        tracing::error!("Failed to get base calendar: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = filter.replace_all(&response, "");

    Ok(response.to_string())
}

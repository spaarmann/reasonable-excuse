use std::net::SocketAddr;

use axum::{routing::get, Router};
use miette::{IntoDiagnostic, Result, WrapErr};
use tracing_subscriber::{prelude::*, EnvFilter};

#[derive(knuffel::Decode)]
struct Config {
    #[knuffel(child, unwrap(argument))]
    address: String,
    #[knuffel(child)]
    upload: UploadConfig,
}

#[derive(knuffel::Decode)]
struct UploadConfig {
    #[knuffel(child, unwrap(argument))]
    route: String,
}

fn read_config() -> Result<Config> {
    let path = "./config.kdl";
    let text = std::fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to read config file at {}", path))?;
    let config = knuffel::parse::<Config>(path, &text).wrap_err("Failed to parse config file")?;
    Ok(config)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive("reasonable_excuse=trace".parse().unwrap())
                .from_env()
                .unwrap(),
        )
        .init();

    let config = read_config()?;

    let app = Router::new().route(&config.upload.route, get(upload_get));

    let addr = config
        .address
        .parse::<SocketAddr>()
        .into_diagnostic()
        .wrap_err_with(|| format!("Could not parse server address: {}", config.address))?;

    tracing::info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .into_diagnostic()
}

#[tracing::instrument]
async fn upload_get() -> &'static str {
    tracing::trace!("GET upload endpoint");
    "POST to this address to upload files"
}

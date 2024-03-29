use std::net::SocketAddr;

use axum::{
    http::{HeaderValue, Method},
    Router,
};
use miette::{IntoDiagnostic, Result, WrapErr};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing_subscriber::{prelude::*, EnvFilter};

mod calendar;
mod firefly_shortcuts;
mod upload;

#[derive(knuffel::Decode, Debug)]
struct Config {
    #[knuffel(child, unwrap(argument))]
    address: String,
    #[knuffel(child, unwrap(argument))]
    allow_origin: Option<String>,
    #[knuffel(child)]
    upload: upload::Config,
    #[knuffel(child)]
    firefly_shortcuts: firefly_shortcuts::Config,
    #[knuffel(child)]
    calendar: calendar::Config,
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
            EnvFilter::try_new(
                std::env::var("RUST_LOG").unwrap_or("info,reasonable_excuse=trace".to_string()),
            )
            .unwrap(),
        )
        .init();

    let config = read_config()?;
    tracing::info!("Starting with config {:?}", config);

    let app = Router::new();
    let app = upload::setup(config.upload, app).context("set up upload module")?;
    let app = firefly_shortcuts::setup(config.firefly_shortcuts, app)
        .context("set up firefly_shortcuts module")?;
    let mut app = calendar::setup(config.calendar, app).context("set up calendar module")?;

    if let Some(allow_origin) = &config.allow_origin {
        app = app.layer(
            CorsLayer::new()
                .allow_methods([Method::GET, Method::PUT])
                .allow_origin(
                    allow_origin
                        .parse::<HeaderValue>()
                        .into_diagnostic()
                        .context("parse allow-origin value")?,
                ),
        );
    }

    let addr = config
        .address
        .parse::<SocketAddr>()
        .into_diagnostic()
        .wrap_err_with(|| format!("Could not parse server address: {}", config.address))?;

    tracing::info!("listening on {}", addr);
    let listener = TcpListener::bind(addr)
        .await
        .into_diagnostic()
        .wrap_err("Could not bind to address!")?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .into_diagnostic()
}

async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("signal received, starting graceful shutdown");
}

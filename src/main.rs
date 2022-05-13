use std::net::SocketAddr;

use axum::{routing::get, Router};
use tracing_subscriber::{prelude::*, EnvFilter};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive("reasonable_excuse=trace".parse().unwrap())
                .from_env()
                .unwrap(),
        )
        .init();

    let app = Router::new().route("/", get(root));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[tracing::instrument]
async fn root() -> &'static str {
    tracing::trace!("root requested");
    "Hello World!"
}

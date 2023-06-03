use axum::{
    extract::{ConnectInfo, Multipart},
    http::StatusCode,
    Router,
};
use std::net::SocketAddr;

pub fn setup(app: Router) -> miette::Result<Router> {
    Ok(app.route("/pcs", axum::routing::post(post)))
}

#[tracing::instrument]
async fn post(
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    body: String,
) -> Result<String, StatusCode> {
    tracing::info!("PCS request!");

    Ok("Thanks!".to_string())
}

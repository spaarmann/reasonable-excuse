use axum::{extract::ConnectInfo, http::StatusCode, Extension, Router};
use std::sync::{Arc, RwLock};
use std::{net::SocketAddr, time::Instant};

struct Request {
    body: String,
    time: Instant,
}

struct State {
    last_requests: Vec<Request>,
}

pub fn setup(app: Router) -> miette::Result<Router> {
    Ok(app
        .route("/pcs", axum::routing::get(get))
        .route("/pcs", axum::routing::post(post))
        .layer(Extension(Arc::new(RwLock::new(State {
            last_requests: Vec::new(),
        })))))
}

#[tracing::instrument(skip(state))]
async fn get(
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    Extension(state): Extension<Arc<RwLock<State>>>,
) -> Result<String, StatusCode> {
    tracing::info!("PCS GET request");

    let state = state
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let now = Instant::now();

    let mut out = String::new();
    for req in &state.last_requests {
        let time = now - req.time;
        let formatted = format!("[{:?} ago] {}\n\n", time, req.body);
        out.push_str(&formatted);
    }

    Ok(out)
}

#[tracing::instrument(skip(state))]
async fn post(
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    Extension(state): Extension<Arc<RwLock<State>>>,
    body: String,
) -> Result<String, StatusCode> {
    tracing::info!("PCS POST request");

    let request = Request {
        body,
        time: Instant::now(),
    };

    let mut state = state
        .write()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.last_requests.push(request);
    if state.last_requests.len() > 50 {
        state.last_requests.drain(..10);
    }

    Ok("Thanks!".to_string())
}

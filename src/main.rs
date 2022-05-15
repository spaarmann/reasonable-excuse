use std::io::ErrorKind;
use std::sync::Arc;
use std::{net::SocketAddr, path::PathBuf};

use axum::extract::ConnectInfo;
use axum::{
    body::Bytes,
    extract::Multipart,
    http::StatusCode,
    routing::{get, post},
    Extension, Router,
};
use miette::{miette, IntoDiagnostic, Result, WrapErr};
use tokio::fs::OpenOptions;
use tracing::Instrument;
use tracing_subscriber::{prelude::*, EnvFilter};

#[derive(knuffel::Decode, Debug)]
struct Config {
    #[knuffel(child, unwrap(argument))]
    address: String,
    #[knuffel(child)]
    upload: UploadConfig,
}

#[derive(knuffel::Decode, Debug)]
struct UploadConfig {
    #[knuffel(child, unwrap(argument))]
    route: String,
    #[knuffel(child, unwrap(argument))]
    target_dir: PathBuf,
    #[knuffel(child, unwrap(argument))]
    filename_length: usize,
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

    let upload_config = Arc::new(config.upload);
    let upload_target_meta = std::fs::metadata(&upload_config.target_dir)
        .into_diagnostic()
        .wrap_err("Failed to check metadata of upload target dir")?;
    if !upload_target_meta.is_dir() {
        return Err(miette!(
            "Upload target path {} is not a directory!",
            upload_config.target_dir.display()
        ));
    }

    let app = Router::new()
        .route(&upload_config.route, get(upload_get))
        .route(&upload_config.route, post(upload_post))
        .layer(Extension(upload_config));

    let addr = config
        .address
        .parse::<SocketAddr>()
        .into_diagnostic()
        .wrap_err_with(|| format!("Could not parse server address: {}", config.address))?;

    tracing::info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .into_diagnostic()
}

#[tracing::instrument]
async fn upload_get(ConnectInfo(client_addr): ConnectInfo<SocketAddr>) -> &'static str {
    tracing::info!("GET upload");
    "POST to this address to upload files"
}

#[tracing::instrument(skip(body, upload_config))]
async fn upload_post(
    body: Multipart,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    Extension(upload_config): Extension<Arc<UploadConfig>>,
) -> Result<String, StatusCode> {
    tracing::info!("Upload request");

    let bytes = get_file_bytes(body).await?;

    loop {
        let name = generate_name(upload_config.filename_length);
        let mut path = upload_config.target_dir.clone();
        path.push(&name);

        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            // happened to get a random path that already exists, try again
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                tracing::error!(path = ?path, error = ?e, "Error opening file for upload");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
            Ok(f) => f,
        };

        tokio::io::copy_buf(&mut bytes.as_ref(), &mut file)
            .instrument(tracing::info_span!("Writing file", path = ?path))
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "Error writing file");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        tracing::info!(path = ?path, "Uploaded file");

        return Ok(name);
    }
}

async fn get_file_bytes(mut body: Multipart) -> Result<Bytes, StatusCode> {
    let field = body
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .ok_or(StatusCode::BAD_REQUEST)?;

    let name = field.name();
    if name != Some("file") {
        return Err(StatusCode::BAD_REQUEST);
    }

    field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)
}

fn generate_name(len: usize) -> String {
    fn num_to_char(num: usize) -> char {
        match num {
            0..=25 => (b'a' + num as u8) as char,
            26..=51 => (b'A' + (num - 26) as u8) as char,
            52..=61 => char::from_digit((num - 52).try_into().unwrap(), 10).unwrap(),
            _ => panic!("invalid num for converting to char!"),
        }
    }

    use rand::prelude::*;
    let mut rng = thread_rng();
    (0..len)
        .map(|_| num_to_char(rng.gen_range(0..=61)))
        .collect()
}

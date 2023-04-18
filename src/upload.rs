use std::{io::ErrorKind, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    body::Bytes,
    extract::{ConnectInfo, DefaultBodyLimit, Multipart, Query},
    http::StatusCode,
    Extension, Router,
};
use miette::{miette, Context, IntoDiagnostic};
use tokio::fs::OpenOptions;
use tracing::Instrument;

#[derive(knuffel::Decode, Debug)]
pub struct Config {
    #[knuffel(child, unwrap(argument))]
    route: String,
    #[knuffel(child, unwrap(argument))]
    target_dir: PathBuf,
    #[knuffel(child, unwrap(argument))]
    filename_length: usize,
}

pub fn setup(config: Config, app: Router) -> miette::Result<Router> {
    let config = Arc::new(config);

    let upload_target_meta = std::fs::metadata(&config.target_dir)
        .into_diagnostic()
        .wrap_err("Failed to check metadata of upload target dir")?;

    if !upload_target_meta.is_dir() {
        return Err(miette!(
            "Upload target path {} is not a directory!",
            config.target_dir.display()
        ));
    }

    Ok(app
        .route(&config.route, axum::routing::get(get))
        .route(&config.route, axum::routing::post(post))
        // This is only accessible internally anyway; I want to be able to upload large files.
        .layer(DefaultBodyLimit::disable())
        .layer(Extension(config)))
}

#[tracing::instrument]
async fn get(ConnectInfo(client_addr): ConnectInfo<SocketAddr>) -> &'static str {
    tracing::info!("GET upload");
    "POST to this address to upload files"
}

#[derive(Debug, serde::Deserialize)]
struct PostParams {
    keep_name: Option<bool>,
}

#[tracing::instrument(skip(body, config))]
async fn post(
    body: Multipart,
    params: Query<PostParams>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    Extension(config): Extension<Arc<Config>>,
) -> Result<String, StatusCode> {
    tracing::info!("Upload request");

    let keep_name = params.keep_name.unwrap_or(false);
    let (original_name, bytes) = get_file_name_and_bytes(body).await?;

    // We want to preserve the original file extension, while replacing the rest of the file name
    // with a random short name.
    let extension = original_name
        .rsplit_once('.')
        .ok_or(StatusCode::BAD_REQUEST)?
        .1;

    loop {
        let name = if keep_name {
            original_name.clone()
        } else {
            let mut name = generate_name(config.filename_length);
            name.push('.');
            name.push_str(&extension);
            name
        };

        let mut path = config.target_dir.clone();
        path.push(&name);

        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            // happened to get a random path that already exists, try again
            Err(e) if e.kind() == ErrorKind::AlreadyExists && !keep_name => continue,
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

async fn get_file_name_and_bytes(mut body: Multipart) -> Result<(String, Bytes), StatusCode> {
    let field = body
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .ok_or(StatusCode::BAD_REQUEST)?;

    let field_name = field.name();
    if field_name != Some("file") {
        return Err(StatusCode::BAD_REQUEST);
    }

    let file_name = field
        .file_name()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    let bytes = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;

    tracing::info!("Got file {} with {} bytes", file_name, bytes.len());
    Ok((file_name, bytes))
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

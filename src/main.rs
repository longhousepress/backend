mod catalog;
mod config;
mod cors;
mod db;
mod download;
mod email;
mod models;
mod state;
mod stripe;
mod submissions;
mod tera;
mod tokens;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, head, post};
use axum::Router;
use figment::Figment;
use figment::providers::Env;
use std::path::PathBuf;
use tokio::net::TcpListener;

use crate::config::{Config, SystemdCreds};
use crate::cors::cors_layer;
use crate::db::load_db;
use crate::state::AppState;
use crate::tera::load_tera;

async fn head_handler() -> StatusCode {
    StatusCode::OK
}

async fn root_redirect() -> impl IntoResponse {
    axum::response::Redirect::permanent("/en/")
}

async fn static_files(
    State(state): State<AppState>,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    let raw_path = uri.path().trim_start_matches('/');

    // Percent-decode first so %2e%2e etc. can't bypass the check
    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8_lossy()
        .into_owned();

    if decoded.contains("..") || decoded.contains('\0') || decoded.contains('\\') {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    }

    let mut full_path = state.public_dir.join(&decoded);
    if full_path.is_dir() {
        full_path.push("index.html");
    }
    match serve_file(&full_path).await {
        Some(response) => response,
        None => {
            let fallback = state.public_dir.join("404.html");
            match tokio::fs::read_to_string(fallback).await {
                Ok(html) => (
                    StatusCode::NOT_FOUND,
                    [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    html,
                )
                    .into_response(),
                Err(_) => (StatusCode::NOT_FOUND, "Not Found").into_response(),
            }
        }
    }
}

async fn serve_file(path: &std::path::Path) -> Option<axum::response::Response> {
    let bytes = tokio::fs::read(path).await.ok()?;
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    let len = bytes.len();
    Some(
        axum::response::Response::builder()
            .status(StatusCode::OK)
            .header(axum::http::header::CONTENT_TYPE, mime)
            .header(axum::http::header::CONTENT_LENGTH, len)
            .body(axum::body::Body::from(bytes))
            .unwrap(),
    )
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Configure Figment to read from Rocket.toml and environment variables
    let figment = Figment::new()
        .merge(Env::prefixed("DRAGON_"))
        .merge(SystemdCreds);

    // Extract config early to use db_path and public_dir
    let config: Config = figment.extract().expect("Failed to extract configuration");

    // Load db and crash immediately if we can't
    let db = load_db(&config.db_path).await.expect("Failed to load database");
    tracing::info!("Database loaded successfully");

    // Initialize Tera templates once at startup
    let tera = load_tera(&config).expect("Failed to load templates");

    let public_dir = PathBuf::from(&config.public_dir);
    let http_client = reqwest::Client::new();

    let app_state = AppState {
        config,
        db,
        tera,
        http_client,
        public_dir,
    };

    // Extract server bind address from figment
    let port: u16 = figment.extract_inner("port").unwrap_or(8000);
    let address: std::net::IpAddr = figment
        .extract_inner("address")
        .unwrap_or_else(|_| [127, 0, 0, 1].into());

    let api_routes = Router::new()
        .route("/books", get(catalog::books))
        .route("/download/{tok}", get(download::download))
        .route("/checkout", post(stripe::checkout::checkout))
        .route("/order/verify", get(stripe::verify_order::verify_order_endpoint))
        .route("/webhook", post(stripe::webhook::stripe_webhook))
        // 11 MB cap: 10 MB file + headroom for multipart framing/fields.
        // DefaultBodyLimit rejects the request before the handler reads any bytes,
        // so a large upload never reaches memory.
        .route("/submit", post(submissions::submit)
                .layer(DefaultBodyLimit::max(11 * 1024 * 1024)),
        );

    let app = Router::new()
        .route("/", head(head_handler).get(root_redirect))
        .nest("/api", api_routes)
        .fallback(static_files)
        .layer(cors_layer())
        .with_state(app_state);

    let listener = TcpListener::bind((address, port))
        .await
        .expect("Failed to bind");
    tracing::info!("Listening on {}:{}", address, port);
    axum::serve(listener, app).await.expect("Server error");
}

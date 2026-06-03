use std::path::PathBuf;

use reqwest::Client;
use sqlx::SqlitePool;
use tera::Tera;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: SqlitePool,
    pub tera: Tera,
    pub http_client: Client,
    pub public_dir: PathBuf,
}

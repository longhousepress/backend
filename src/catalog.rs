use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::state::AppState;
use crate::db::load_books;
use crate::models::{Book, Language};

#[derive(Deserialize)]
pub struct CatalogParams {
    language: Language,
}

pub async fn books(
    State(state): State<AppState>,
    Query(params): Query<CatalogParams>,
) -> Result<Json<Vec<Book>>, StatusCode> {
    match load_books(&state.db, &state.config.static_dir, params.language.as_str()).await {
        Ok(books) => Ok(Json(books)),
        Err(e) => {
            tracing::error!("Failed to load books catalog: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

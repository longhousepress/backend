use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::state::AppState;
use crate::db::load_books;
use crate::models::Book;

pub async fn books(
    State(state): State<AppState>,
) -> Result<Json<Vec<Book>>, StatusCode> {
    match load_books(&state.db, &state.config.static_dir).await {
        Ok(books) => Ok(Json(books)),
        Err(e) => {
            tracing::error!("Failed to load books catalog: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

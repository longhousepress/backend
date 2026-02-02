use rocket::{serde::json::Json, State, http::Status};
use sqlx::SqlitePool;
use tracing::error;
use crate::db::get_book_by_slug;

use crate::models::Book;

#[get("/api/books/<slug>", rank = 2)]
pub async fn book_detail(db: &State<SqlitePool>, slug: String) -> Result<Json<Book>, Status> {
    match get_book_by_slug(&db, &slug).await {
        Ok(Some(book)) => Ok(Json(book)),
        Ok(None) => Err(Status::NotFound),
        Err(e) => {
            error!("Failed to fetch book details for slug '{}': {}", slug, e);
            Err(Status::InternalServerError)
        }
    }
}

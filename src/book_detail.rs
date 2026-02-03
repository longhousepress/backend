use crate::db::get_book_by_slug;
use rocket::{State, http::Status, serde::json::Json};
use sqlx::SqlitePool;

use crate::models::Book;

#[get("/api/books/<slug>", rank = 2)]
pub async fn book_detail(db: &State<SqlitePool>, slug: String) -> Result<Json<Book>, Status> {
    match get_book_by_slug(db, &slug).await {
        Ok(Some(book)) => Ok(Json(book)),
        Ok(None) => Err(Status::NotFound),
        Err(e) => {
            rocket::error!("Failed to fetch book by slug '{}': {}", slug, e);
            Err(Status::InternalServerError)
        }
    }
}

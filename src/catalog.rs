use rocket::{State, http::Status, serde::json::Json};
use sqlx::SqlitePool;

use crate::config::Config;
use crate::db::load_books;
use crate::models::Book;

#[get("/books", rank = 1)]
pub async fn books(db: &State<SqlitePool>, config: &State<Config>) -> Result<Json<Vec<Book>>, Status> {
    match load_books(db, &config.static_dir).await {
        Ok(books) => Ok(Json(books)),
        Err(e) => {
            rocket::error!("Failed to load books catalog: {}", e);
            Err(Status::InternalServerError)
        }
    }
}

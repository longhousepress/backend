use rocket::{State, http::Status, serde::json::Json};
use sqlx::SqlitePool;

use crate::db::load_books;
use crate::models::Book;

#[get("/books", rank = 1)]
pub async fn books(db: &State<SqlitePool>) -> Result<Json<Vec<Book>>, Status> {
    match load_books(db).await {
        Ok(books) => Ok(Json(books)),
        Err(e) => {
            rocket::error!("Failed to load books catalog: {}", e);
            Err(Status::InternalServerError)
        }
    }
}

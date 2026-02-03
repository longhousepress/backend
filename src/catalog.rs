use rocket::{State, http::Status, serde::json::Json};
use sqlx::SqlitePool;

use crate::db::load_books;
use crate::models::Book;

#[derive(FromForm)]
pub struct LangQuery {
    pub lang: Option<String>,
}

#[get("/api/books?<query..>", rank = 1)]
pub async fn books(db: &State<SqlitePool>, query: LangQuery) -> Result<Json<Vec<Book>>, Status> {
    let lang = query.lang.as_deref();
    match load_books(db, lang).await {
        Ok(books) => Ok(Json(books)),
        Err(e) => {
            rocket::error!("Failed to load books catalog (lang={:?}): {}", lang, e);
            Err(Status::InternalServerError)
        }
    }
}

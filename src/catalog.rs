use rocket::{serde::json::Json, State, http::Status};
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
    match load_books(&db, lang).await {
        Ok(books) => Ok(Json(books)),
        Err(_) => Err(Status::InternalServerError)
    }
}

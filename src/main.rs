mod db;
mod models;

use rocket::{serde::json::Json, State, http::Status};
use sqlx::SqlitePool;

use crate::db::load_db;
use crate::models::{Book, BookDetail};

#[macro_use] extern crate rocket;

#[derive(FromForm)]
struct LangQuery {
    lang: Option<String>,
}

#[get("/api/books/<slug>", rank = 2)]
async fn book_detail(db: &State<SqlitePool>, slug: String) -> Result<Json<BookDetail>, Status> {
    match db::get_book_by_slug(&db, &slug).await {
        Ok(Some(book)) => Ok(Json(book)),
        Ok(None) => Err(Status::NotFound),
        Err(_) => Err(Status::InternalServerError)
    }
}

#[get("/api/books?<query..>", rank = 1)]
async fn books(db: &State<SqlitePool>, query: LangQuery) -> Result<Json<Vec<Book>>, Status> {
    let lang = query.lang.as_deref();
    match db::load_books(&db, lang).await {
        Ok(books) => Ok(Json(books)),
        Err(_) => Err(Status::InternalServerError)
    }
}

#[launch]
async fn rocket() -> _ {
    // Load the database once at startup
    let db = load_db().await.expect("Failed to load database");

    rocket::build()
        .manage(db)  // Register the pool as managed state
        .mount("/", routes![book_detail, books])
}

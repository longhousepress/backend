mod db;
mod models;

use std::path::PathBuf;
use rocket::{serde::json::Json, State, http::Status};
use sqlx::SqlitePool;

use crate::db::load_db;
use crate::models::{Book, BookDetail};

#[macro_use] extern crate rocket;

#[get("/api/books/<slug..>", rank = 2)]
async fn book_detail(db: &State<SqlitePool>, slug: PathBuf) -> Result<Json<BookDetail>, Status> {
    let slug_str = slug.to_string_lossy();
    eprintln!("DEBUG: Received slug: {:?}", slug_str);
    match db::get_book_by_slug(&db, &slug_str).await {
        Ok(Some(book)) => Ok(Json(book)),
        Ok(None) => Err(Status::NotFound),
        Err(_) => Err(Status::InternalServerError)
    }
}

#[get("/api/books", rank = 1)]
async fn books(db: &State<SqlitePool>) -> Result<Json<Vec<Book>>, Status> {
    match db::load_books(&db).await {
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

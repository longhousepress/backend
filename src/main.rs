mod db;
mod models;

use rocket::{serde::json::Json, State, http::Status};
use sqlx::SqlitePool;

use crate::db::load_db;
use crate::models::Book;

#[macro_use] extern crate rocket;

#[get("/api/books")]
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
        .mount("/", routes![books])
}

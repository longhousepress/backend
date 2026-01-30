mod db;
mod models;
mod stripe;
use rocket::fs::NamedFile;
use rocket::{serde::json::Json, State, http::Status};
use sqlx::SqlitePool;

use crate::db::load_db;
use crate::models::{Book, BookDetail, CheckoutRequest, CheckoutSession};
use crate::stripe::verify;
use rocket_cors::{AllowedOrigins, CorsOptions};

#[macro_use] extern crate rocket;

#[derive(FromForm)]
struct LangQuery {
    lang: Option<String>,
}

#[post("/api/checkout", data = "<request>")]
async fn create_checkout_session(db: &State<SqlitePool>, request: Json<CheckoutRequest>) -> Result<Json<CheckoutSession>, Status> {
    // take ownership of the parsed request body
    let req = request.into_inner();
    match stripe::create_checkout_session(&db, &req).await {
        Ok(s) => Ok(Json(s)),
        Err(_) => Err(Status::InternalServerError)
    }
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

#[get("/api/download/<tok>")]
async fn download(tok: &str) -> Result<NamedFile, Status> {
    verify(tok).map_err(|e| {
        eprintln!("verify error: {:?}", e);   // log it
        Status::Gone
    })?;
    NamedFile::open("static/Astrophel and Stella.epub")
        .await
        .map_err(|e| {
            eprintln!("file error: {:?}", e);
            Status::InternalServerError
        })
}

#[launch]
async fn rocket() -> _ {
	let cors = CorsOptions {
        allowed_origins: AllowedOrigins::all(), // dev only; restrict in prod
        ..Default::default()
    }
    .to_cors()
    .expect("CORS setup");

    // Load the database once at startup
    let db = load_db().await.expect("Failed to load database");

    rocket::build()
        .manage(db)
        .attach(cors)  // Register the pool as managed state
        .mount("/", routes![create_checkout_session, book_detail, books, download])
}

mod db;
mod models;
mod stripe;
use rocket::fs::NamedFile;
use rocket::{serde::json::Json, State, http::Status};
use sqlx::SqlitePool;
use serde_json::json;

use crate::db::load_db;
use crate::models::{Book, BookDetail, CheckoutRequest, CheckoutSession};
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
async fn download(db: &State<SqlitePool>, tok: &str) -> Result<NamedFile, Status> {
    // Treat `tok` as a download token: verify signature and serve the underlying file.
    crate::stripe::verify(tok).map_err(|_| Status::Gone)?;

    // Resolve the token -> edition -> file_path, ensuring token is not expired (if expiry present)
    let file_row = sqlx::query!(
        "SELECT e.file_path FROM download_tokens dt \
         INNER JOIN editions e ON dt.edition_id = e.id \
         WHERE dt.token = ? AND (dt.expires_at IS NULL OR dt.expires_at > strftime('%Y-%m-%dT%H:%M:%SZ','now')) LIMIT 1",
        tok
    )
    .fetch_optional(db.inner())
    .await
    .map_err(|e| { eprintln!("db lookup error: {:?}", e); Status::InternalServerError })?;

    let file_path = match file_row {
        Some(r) => r.file_path,
        None => return Err(Status::NotFound),
    };

    NamedFile::open(file_path).await.map_err(|e| {
        eprintln!("file open error: {:?}", e);
        Status::InternalServerError
    })
}

#[get("/api/downloads/order/<order_id>")]
async fn downloads_for_order(db: &State<SqlitePool>, order_id: i64) -> Result<Json<serde_json::Value>, Status> {
    // Ensure the order exists and is marked paid (paid == 1)
    let row_opt = sqlx::query!("SELECT paid FROM orders WHERE id = ?", order_id)
        .fetch_optional(db.inner())
        .await
        .map_err(|e| { eprintln!("db error: {:?}", e); Status::InternalServerError })?;

    let paid_val = match row_opt {
        Some(r) => r.paid,
        None => return Err(Status::NotFound),
    };

    if paid_val != Some(1) {
        return Err(Status::Forbidden);
    }

    // Fetch tokens and edition titles for this order
    let rows = sqlx::query!(
        "SELECT dt.token, dt.expires_at, e.title \
         FROM download_tokens dt \
         INNER JOIN editions e ON dt.edition_id = e.id \
         WHERE dt.order_id = ?",
        order_id
    )
    .fetch_all(db.inner())
    .await
    .map_err(|e| { eprintln!("db error: {:?}", e); Status::InternalServerError })?;

    let mut list = Vec::with_capacity(rows.len());
    for r in rows {
        let token = r.token;
        let url = format!("/api/download/{}", token);
        list.push(json!({
            "token": token,
            "url": url,
            "title": r.title,
            "expires_at": r.expires_at
        }));
    }

    Ok(Json(json!(list)))
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
        .mount("/", routes![create_checkout_session, book_detail, books, download, downloads_for_order, stripe::verify_order_endpoint, stripe::stripe_webhook])
}

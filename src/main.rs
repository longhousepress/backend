mod db;
mod models;
mod stripe;

use rocket::fs::NamedFile;
use rocket::{serde::json::Json, State, http::Status};
use rocket::response::{Responder, Result as RespResult};
use rocket::Request;
use sqlx::SqlitePool;
use serde_json::json;
use std::path::Path;


use crate::db::load_db;
use crate::stripe::checkout::{CheckoutRequest, CheckoutSession};
use crate::stripe::download::verify;
use crate::models::Book;
use rocket_cors::{AllowedOrigins, CorsOptions};

const REQUIRED_VARS: [&str; 3] = ["TOKEN_KEY", "STRIPE_API_KEY", "STRIPE_WEBHOOK_SECRET"];

#[macro_use] extern crate rocket;

#[derive(FromForm)]
struct LangQuery {
    lang: Option<String>,
}

#[post("/api/checkout", data = "<request>")]
async fn checkout(db: &State<SqlitePool>, request: Json<CheckoutRequest>) -> Result<Json<CheckoutSession>, Status> {
    // take ownership of the parsed request body
    let req = request.into_inner();
    match stripe::checkout::create_checkout_session(&db, &req).await {
        Ok(s) => Ok(Json(s)),
        Err(e) => {
        	eprintln!("Error when creating a checkout session: {}", e);
        	Err(Status::InternalServerError)
        }
    }
}

#[get("/api/books/<slug>", rank = 2)]
async fn book_detail(db: &State<SqlitePool>, slug: String) -> Result<Json<Book>, Status> {
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


#[derive(Debug)]
struct DownloadResponder {
    file: NamedFile,
    filename: String,
}

impl<'r> Responder<'r, 'static> for DownloadResponder {
    fn respond_to(self, req: &'r Request<'_>) -> RespResult<'static> {
        let mut response = self.file.respond_to(req)?;
        response.set_raw_header("Content-Disposition", format!("attachment; filename=\"{}\"", self.filename));
        Ok(response)
    }
}

#[get("/api/download/<tok>")]
async fn download(db: &State<SqlitePool>, tok: &str) -> Result<DownloadResponder, Status> {
    // Check if the token is valid
    match verify(tok) {
        Ok(_) => (),
        Err(_) => return Err(Status::Gone)
    };

    // It is valid, get the file that it gives access to
    let file_row = sqlx::query!(
        "SELECT f.file_path FROM download_tokens dt \
         INNER JOIN files f ON dt.file_id = f.id \
         WHERE dt.token = ? LIMIT 1",
        tok
    )
    .fetch_optional(db.inner())
    .await
    .map_err(|e| { eprintln!("db lookup error: {:?}", e); Status::InternalServerError })?;

    let file_path = match file_row {
        Some(r) => r.file_path,
        None => return Err(Status::NotFound),
    };

    let named_file = NamedFile::open(&file_path).await.map_err(|e| {
        eprintln!("file open error: {:?}", e);
        Status::InternalServerError
    })?;

    // Extract filename from path
    let filename = Path::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .to_string();

    Ok(DownloadResponder { file: named_file, filename })
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
         INNER JOIN files f ON dt.file_id = f.id \
         INNER JOIN editions e ON f.edition_id = e.id \
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
	// Load .env and crash immediately if it's not there
	dotenvy::dotenv().expect("Failed to load .env");

	// Crash if any of the expected env vars are missing
	if let Some(missing) = REQUIRED_VARS.iter().find(|v| std::env::var(v).is_err()) {
    	panic!("Missing required environment variable: {}", missing);
	}

    // Load db and crash immediately if we can't
    let db = load_db().await.expect("Failed to load database");

    // Set CORS
	let cors = CorsOptions {
           allowed_origins: AllowedOrigins::all(), // dev only; restrict in prod
           ..Default::default()
       }
       .to_cors()
       .expect("CORS setup");

	// And launch
    rocket::build()
        .manage(db)
        .attach(cors)  // Register the pool as managed state
        .mount("/", routes![stripe::download::verify_order_endpoint, checkout, book_detail, books, download, downloads_for_order, stripe::webhook::stripe_webhook])
}

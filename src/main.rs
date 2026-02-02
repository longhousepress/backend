mod db;
mod models;
mod stripe;

use rocket::fs::NamedFile;
use rocket::{serde::json::Json, State, http::Status};
use rocket::response::{Responder, Result as RespResult};
use rocket::Request;
use sqlx::SqlitePool;
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
async fn download(tok: &str) -> Result<DownloadResponder, Status> {
    // Verify the token and extract the filepath from its payload
    let file_path = match verify(tok) {
        Ok(p) => p,
        Err(_) => return Err(Status::Gone),
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
        .mount("/", routes![stripe::download::verify_order_endpoint, checkout, book_detail, books, download, stripe::webhook::stripe_webhook])
}

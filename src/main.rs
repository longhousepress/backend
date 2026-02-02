mod config;
mod db;
mod models;
mod stripe;
mod download;
mod catalog;
mod book_detail;

use rocket_cors::{AllowedOrigins, CorsOptions};
use std::collections::HashSet;

use crate::config::Config;
use crate::db::load_db;

#[macro_use] extern crate rocket;
#[launch]
async fn rocket() -> _ {
	// Load .env and crash immediately if it's not there
	dotenvy::dotenv().expect("Failed to load .env");

	// Load config and crash immediately if any required env vars are missing
	let config = Config::from_env();

    // Load db and crash immediately if we can't
    let db = load_db().await.expect("Failed to load database");

    // Set CORS
	let cors = CorsOptions {
           allowed_origins: AllowedOrigins::all(), // dev only; restrict in prod
           expose_headers: vec!["X-Order-Id".into()].into_iter().collect::<HashSet<String>>(),
           ..Default::default()
       }
       .to_cors()
       .expect("CORS setup");

	// And launch
    rocket::build()
        .manage(config)
        .manage(db)
        .attach(cors)
        .mount("/", routes![
            stripe::verify_order::verify_order_endpoint,
            stripe::checkout::checkout,
            book_detail::book_detail,
            catalog::books,
            download::download,
            stripe::webhook::stripe_webhook
        ])
}

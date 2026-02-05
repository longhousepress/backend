mod book_detail;
mod catalog;
mod config;
mod db;
mod download;
mod email;
mod models;
mod stripe;

use rocket_cors::{AllowedOrigins, CorsOptions};
use std::collections::HashSet;
use tera::Tera;


use crate::config::Config;
use crate::db::load_db;

#[macro_use]
extern crate rocket;
#[launch]
async fn rocket() -> _ {
    // Load .env and crash immediately if it's not there
    dotenvy::dotenv().expect("Failed to load .env");

    // Load config and crash immediately if any required env vars are missing
    let config = Config::from_env();

    rocket::info!("Dragon backend starting up");

    // Load db and crash immediately if we can't
    let db = load_db().await.expect("Failed to load database");
    rocket::info!("Database loaded successfully");

    // Initialize Tera templates once at startup and manage it in Rocket state.
    let tera = Tera::new("templates/**/*.html.tera")
        .expect("Failed to initialize Tera templates");

    // Set CORS from config
    let allowed_origins = AllowedOrigins::some_exact(&config.allowed_origins);
    let cors = CorsOptions {
        allowed_origins,
        expose_headers: vec!["X-Order-Id".into()]
            .into_iter()
            .collect::<HashSet<String>>(),
        ..Default::default()
    }
    .to_cors()
    .expect("CORS setup");

    rocket::info!("CORS configured for origins: {:?}", config.allowed_origins);

    // And launch
    rocket::build()
        .manage(tera)
        .manage(config)
        .manage(db)
        .attach(cors)
        .mount(
            "/",
            routes![
                stripe::verify_order::verify_order_endpoint,
                stripe::checkout::checkout,
                book_detail::book_detail,
                catalog::books,
                download::download,
                stripe::webhook::stripe_webhook
            ],
        )
}

mod book_detail;
mod catalog;
mod config;
mod db;
mod download;
mod email;
mod models;
mod stripe;

use rocket_cors::{AllowedOrigins, CorsOptions, AllowedHeaders, AllowedMethods};
use rocket::http::Method;
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

    // Load config from environment and exit with non-zero status if any required vars are
    // missing or invalid. Config::from_env now returns a Result, so handle errors here.
    let config = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            // Rocket logging isn't initialized yet; print to stderr and exit non-zero so
            // process managers can detect startup failure.
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    rocket::info!("Dragon backend starting up");

    // Load db and crash immediately if we can't
    let db = load_db().await.expect("Failed to load database");
    rocket::info!("Database loaded successfully");

    // Initialize Tera templates once at startup and manage it in Rocket state.
    let tera = Tera::new("templates/**/*.html.tera")
        .expect("Failed to initialize Tera templates");

    // Set CORS from config
    let allowed_origins = AllowedOrigins::some_exact(&config.allowed_origins);
    let allowed_methods: AllowedMethods = vec![
        Method::Get,
        Method::Post,
    ]
    .into_iter()
    .map(From::from)
    .collect();

    let cors = CorsOptions {
        allowed_origins,
        allowed_methods,
        allowed_headers: AllowedHeaders::all(),
        allow_credentials: true,
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

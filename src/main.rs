mod config;
mod db;
mod models;
mod stripe;
mod download;
mod catalog;
mod book_detail;
mod email;

use rocket_cors::{AllowedOrigins, CorsOptions};
use std::collections::HashSet;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing::info;

use crate::config::Config;
use crate::db::load_db;

#[macro_use] extern crate rocket;
#[launch]
async fn rocket() -> _ {
	// Load .env and crash immediately if it's not there
	dotenvy::dotenv().expect("Failed to load .env");

	// Load config and crash immediately if any required env vars are missing
	let config = Config::from_env();

	// Initialize logging to file
	let file_appender = RollingFileAppender::new(Rotation::DAILY, &config.log_path, "dragon.log");
	let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
	
	tracing_subscriber::registry()
		.with(fmt::layer().with_writer(non_blocking).with_ansi(false))
		.with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("backend=info")))
		.init();

	info!("Dragon backend starting up");
	info!("Logging initialized to: {}", config.log_path);

    // Load db and crash immediately if we can't
    let db = load_db().await.expect("Failed to load database");
    info!("Database loaded successfully");

    // Set CORS from config
	let allowed_origins = AllowedOrigins::some_exact(&config.allowed_origins);
	let cors = CorsOptions {
           allowed_origins,
           expose_headers: vec!["X-Order-Id".into()].into_iter().collect::<HashSet<String>>(),
           ..Default::default()
       }
       .to_cors()
       .expect("CORS setup");
	
	info!("CORS configured for origins: {:?}", config.allowed_origins);

	// And launch
    rocket::build()
        .manage(config)
        .manage(db)
        .manage(guard)
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

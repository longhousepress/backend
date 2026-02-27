mod book_detail;
mod catalog;
mod config;
mod cors;
mod db;
mod download;
mod email;
mod models;
mod stripe;
mod tokens;

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use rocket::fairing::AdHoc;
use rocket::fs::FileServer;
use tera::Tera;

use crate::config::Config;
use crate::cors::setup_cors;
use crate::db::load_db;

#[macro_use]
extern crate rocket;

#[launch]
async fn rocket() -> _ {
    // Load db and crash immediately if we can't
    let db = load_db().await.expect("Failed to load database");
    rocket::info!("Database loaded successfully");

    // Initialize Tera templates once at startup and manage it in Rocket state.
    let tera = Tera::new("templates/**/*.html.tera").expect("Failed to initialize Tera templates");

    // Configure Figment to read from Rocket.toml and environment variables
    let figment = Figment::from(rocket::Config::default())
        .merge(Toml::file("Rocket.toml").nested())
        .merge(Env::prefixed("DRAGON_"));

    rocket::custom(figment)
        .manage(tera)
        .manage(db)
        .attach(AdHoc::config::<Config>())
        .attach(AdHoc::on_ignite("CORS Setup", setup_cors))
        .mount(
            "/api",
            routes![
                stripe::verify_order::verify_order_endpoint,
                stripe::checkout::checkout,
                book_detail::book_detail,
                catalog::books,
                download::download,
                stripe::webhook::stripe_webhook
            ],
        )
        .mount("/", FileServer::from("public"))
}

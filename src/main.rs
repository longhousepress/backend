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

use figment::providers::{Env, Format, Toml};
use figment::{Figment, Profile};
use rocket::fairing::AdHoc;
use tera::Tera;

use crate::config::Config;
use crate::cors::setup_cors;
use crate::db::load_db;

#[macro_use]
extern crate rocket;

#[launch]
async fn rocket() -> _ {
    // Load .env and crash immediately if it's not there
    dotenvy::dotenv().expect("Failed to load .env");

    rocket::info!("Dragon backend starting up");

    // Load db and crash immediately if we can't
    let db = load_db().await.expect("Failed to load database");
    rocket::info!("Database loaded successfully");

    // Initialize Tera templates once at startup and manage it in Rocket state.
    let tera = Tera::new("templates/**/*.html.tera").expect("Failed to initialize Tera templates");

    // Configure Figment to read from Rocket.toml and environment variables
    let figment = Figment::from(rocket::Config::default())
        .merge(Toml::file("Rocket.toml").nested())
        .merge(Env::raw())
        .select(Profile::from_env_or("ROCKET_PROFILE", "default"));

    rocket::custom(figment)
        .manage(tera)
        .manage(db)
        .attach(AdHoc::config::<Config>())
        .attach(AdHoc::on_ignite("CORS Setup", setup_cors))
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

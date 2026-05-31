mod catalog;
mod config;
mod cors;
mod db;
mod download;
mod email;
mod models;
mod stripe;
mod submissions;
mod tera;
mod tokens;
mod head;

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use rocket::fairing::AdHoc;
use rocket::fs::FileServer;
use rocket::Request;

#[catch(404)]
fn not_found(req: &Request) -> rocket::http::Status {
    debug!("404 {} {}", req.method(), req.uri());
    rocket::http::Status::NotFound
}

use crate::config::{Config, SystemdCreds};
use crate::cors::setup_cors;
use crate::db::load_db;
use crate::tera::load_tera;

#[macro_use]
extern crate rocket;

#[launch]
async fn rocket() -> _ {
    // Configure Figment to read from Rocket.toml and environment variables
    let figment = Figment::from(rocket::Config::default())
        .merge(Toml::file("Rocket.toml").nested())
        .merge(Env::prefixed("DRAGON_"))
        .merge(SystemdCreds);

    // Extract config early to use db_path and public_dir
    let config: Config = figment.extract().expect("Failed to extract configuration");

    // Load db and crash immediately if we can't
    let db = load_db(&config.db_path).await.expect("Failed to load database");
    rocket::info!("Database loaded successfully");

    // Initialize Tera templates once at startup and manage it in Rocket state.
    let tera = load_tera(&config).expect("Failed to load templates");

    let public_dir = config.public_dir.clone();
    let http_client = reqwest::Client::new();

    rocket::custom(figment)
        .manage(tera)
        .manage(db)
        .manage(http_client)
        .attach(AdHoc::config::<Config>())
        .attach(AdHoc::on_ignite("CORS Setup", setup_cors))
        .mount("/", routes![head::head])
        .mount(
            "/api",
            routes![
                stripe::verify_order::verify_order_endpoint,
                stripe::checkout::checkout,
                catalog::books,
                download::download,
                stripe::webhook::stripe_webhook,
                submissions::submit,
            ],
        )
        .mount("/", FileServer::from(public_dir))
        .register("/", catchers![not_found])
}

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

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use std::path::PathBuf;

use rocket::fairing::AdHoc;
use rocket::fs::NamedFile;
use rocket::http::Status;
use rocket::Request;

#[catch(404)]
fn not_found(req: &Request) -> String {
    format!("{} does not exist.", req.uri())
}

use crate::config::{Config, SystemdCreds};
use crate::cors::setup_cors;
use crate::db::load_db;
use crate::tera::load_tera;

#[head("/")]
fn head() -> rocket::http::Status {
    rocket::http::Status::Ok
}

// Custom wrapper over NamedFile instead of using the built-in FileServer so
// that non-existent files (bot spam mainly) don't log a critical warning.
#[get("/<path..>", rank = 10)]
async fn static_files(root: &rocket::State<PathBuf>, path: PathBuf) -> Result<NamedFile, (Status, ())> {
    let mut full_path = root.join(&path);
    if full_path.is_dir() {
        full_path.push("index.html");
    }
    NamedFile::open(full_path).await.map_err(|_| (Status::NotFound, ()))
}

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

    let public_dir = PathBuf::from(&config.public_dir);
    let http_client = reqwest::Client::new();

    rocket::custom(figment)
        .manage(tera)
        .manage(db)
        .manage(http_client)
        .manage(public_dir)
        .attach(AdHoc::config::<Config>())
        .attach(AdHoc::on_ignite("CORS Setup", setup_cors))
        .mount("/", routes![head, static_files])
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
        .register("/", catchers![not_found])
}

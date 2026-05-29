mod catalog;
mod config;
mod cors;
mod db;
mod download;
mod email;
mod models;
mod stripe;
mod submissions;
mod tokens;
mod head;

use figment::value::{Dict, Map};
use figment::{Error, Figment, Metadata, Profile, Provider};
use figment::providers::{Env, Format, Toml};

struct SystemdCreds;

impl Provider for SystemdCreds {
    fn metadata(&self) -> Metadata {
        Metadata::named("systemd credentials")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut dict = Dict::new();
        let Ok(cred_dir) = std::env::var("CREDENTIALS_DIRECTORY") else {
            return Ok(Map::from([(Profile::Default, dict)]));
        };
        let creds = [
            ("dragon-token_key",             "token_key"),
            ("dragon-stripe_api_key",        "stripe_api_key"),
            ("dragon-stripe_webhook_secret", "stripe_webhook_secret"),
            ("dragon-resend_api_key",        "resend_api_key"),
            ("dragon-submissions_to_email",  "submissions_to_email"),
        ];
        for (file, key) in &creds {
            let path = std::path::Path::new(&cred_dir).join(file);
            if let Ok(val) = std::fs::read_to_string(path) {
                dict.insert(key.to_string(), val.trim().to_string().into());
            }
        }
        Ok(Map::from([(Profile::Default, dict)]))
    }
}
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
    let tera = Tera::new(&format!("{}/**/*.html.tera", config.templates_dir)).expect("Failed to initialize Tera templates");

    let public_dir = config.public_dir.clone();

    rocket::custom(figment)
        .manage(tera)
        .manage(db)
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
}

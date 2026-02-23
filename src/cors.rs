use crate::config::Config;
use rocket::http::Method;
use rocket::{Build, Rocket};
use rocket_cors::{AllowedHeaders, AllowedMethods, AllowedOrigins, CorsOptions};
use std::collections::HashSet;

// Fairing to set up CORS based on the extracted config
// Only applies CORS if allowed_origins is not empty (for dev)
pub async fn setup_cors(rocket: Rocket<Build>) -> Rocket<Build> {
    let config = rocket
        .state::<Config>()
        .expect("Config should be managed at this point");

    // Only set up CORS if we have allowed origins (for dev environments)
    if config.allowed_origins.is_empty() {
        rocket::info!("CORS disabled - no allowed_origins configured");
        return rocket;
    }

    let allowed_origins = AllowedOrigins::some_exact(&config.allowed_origins);
    let allowed_methods: AllowedMethods = vec![Method::Get, Method::Post]
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

    rocket.attach(cors)
}

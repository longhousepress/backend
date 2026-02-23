use rocket::http::Method;
use rocket::{Build, Rocket};
use rocket_cors::{AllowedHeaders, AllowedOrigins, CorsOptions};

// Sets up CORS based on profile configuration
// In debug profile: CORS is enabled for development origins (e.g., http://localhost:5173)
// In release profile: CORS is disabled - frontend is served from public/ directory
pub async fn setup_cors(rocket: Rocket<Build>) -> Rocket<Build> {
    if cfg!(not(debug_assertions)) {
        rocket::info!("CORS disabled in release profile");
        return rocket;
    }

    let cors = CorsOptions {
        allowed_origins: AllowedOrigins::all(),
        allowed_methods: vec![Method::Get, Method::Post]
            .into_iter()
            .map(From::from)
            .collect(),
        allowed_headers: AllowedHeaders::all(),
        allow_credentials: false, // can't use credentials with wildcard origin
        ..Default::default()
    }
    .to_cors()
    .expect("CORS setup");

    rocket::info!("CORS enabled for all origins (dev profile)");
    rocket.attach(cors)
}

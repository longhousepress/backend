use http::Method;
use tower_http::cors::{Any, CorsLayer};

// Sets up CORS based on profile configuration
// In debug profile: CORS is enabled for development origins (e.g., http://localhost:5173)
// In release profile: CORS is disabled - frontend is served from public/ directory
pub fn cors_layer() -> CorsLayer {
    if cfg!(debug_assertions) {
        tracing::info!("CORS enabled for all origins (dev profile)");
        return CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers(Any);
    }

    tracing::info!("CORS disabled in release profile");
    CorsLayer::new()
}

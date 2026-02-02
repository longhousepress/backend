use std::env;

// Application configuration loaded from environment variables.
// All fields are required and the application will panic at startup if any are missing.
#[derive(Debug, Clone)]
pub struct Config {
    pub token_key: String,
    pub stripe_api_key: String,
    pub stripe_webhook_secret: String,
    pub log_path: String,
}

impl Config {
    // Load configuration from environment variables.
    // Panics if any required variable is missing.
    pub fn from_env() -> Self {
        Self {
            token_key: Self::get_required("TOKEN_KEY"),
            stripe_api_key: Self::get_required("STRIPE_API_KEY"),
            stripe_webhook_secret: Self::get_required("STRIPE_WEBHOOK_SECRET"),
            log_path: Self::get_required("LOG_PATH"),
        }
    }

    fn get_required(key: &str) -> String {
        env::var(key).unwrap_or_else(|_| panic!("Missing required environment variable: {}", key))
    }
}

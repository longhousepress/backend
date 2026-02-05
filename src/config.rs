use std::env;
use anyhow::{anyhow, Result};

// Application configuration loaded from environment variables.
// All fields are required and the application will panic at startup if any are missing.
#[derive(Debug, Clone)]
pub struct Config {
    pub token_key: String,
    pub stripe_api_key: String,
    pub stripe_webhook_secret: String,
    pub allowed_origins: Vec<String>,
    pub stripe_success_url: String,
    pub stripe_cancel_url: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_from_email: String,
    pub smtp_from_name: String,
    pub base_url: String,
}

impl Config {
    // Load configuration from environment variables.
    // Returns Ok(Config) or an error describing the missing/invalid variable.
    pub fn from_env() -> Result<Self> {
        let allowed_origins_str = Self::get_required("ALLOWED_ORIGINS")?;
        let allowed_origins = allowed_origins_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let smtp_port_str = Self::get_required("SMTP_PORT")?;
        let smtp_port = smtp_port_str
            .parse::<u16>()
            .map_err(|_| anyhow!("SMTP_PORT must be a valid port number: {}", smtp_port_str))?;

        Ok(Self {
            token_key: Self::get_required("TOKEN_KEY")?,
            stripe_api_key: Self::get_required("STRIPE_API_KEY")?,
            stripe_webhook_secret: Self::get_required("STRIPE_WEBHOOK_SECRET")?,
            allowed_origins,
            stripe_success_url: Self::get_required("STRIPE_SUCCESS_URL")?,
            stripe_cancel_url: Self::get_required("STRIPE_CANCEL_URL")?,
            smtp_host: Self::get_required("SMTP_HOST")?,
            smtp_port,
            smtp_username: Self::get_required("SMTP_USERNAME")?,
            smtp_password: Self::get_required("SMTP_PASSWORD")?,
            smtp_from_email: Self::get_required("SMTP_FROM_EMAIL")?,
            smtp_from_name: Self::get_required("SMTP_FROM_NAME")?,
            base_url: Self::get_required("BASE_URL")?,
        })
    }

    fn get_required(key: &str) -> Result<String> {
        env::var(key).map_err(|_| anyhow!("Missing required environment variable: {}", key))
    }
}

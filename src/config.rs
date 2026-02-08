use rocket::serde::{Deserialize, Serialize};

// Application configuration loaded from environment variables.
// All fields are required and the application will panic at startup if any are missing.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct Config {
    pub token_key: String,
    pub stripe_api_key: String,
    pub stripe_webhook_secret: String,
    #[serde(deserialize_with = "deserialize_comma_separated")]
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

// Custom deserializer for comma-separated strings
fn deserialize_comma_separated<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: rocket::serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let origins = s
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(origins)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            token_key: String::new(),
            stripe_api_key: String::new(),
            stripe_webhook_secret: String::new(),
            allowed_origins: vec![],
            stripe_success_url: String::new(),
            stripe_cancel_url: String::new(),
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_username: String::new(),
            smtp_password: String::new(),
            smtp_from_email: String::new(),
            smtp_from_name: String::new(),
            base_url: String::new(),
        }
    }
}

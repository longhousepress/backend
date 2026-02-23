use rocket::serde::Deserialize;

// All fields are required
#[derive(Debug, Clone, Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct Config {
    pub token_key: String,
    pub stripe_api_key: String,
    pub stripe_webhook_secret: String,
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

use rocket::serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct Config {
    pub token_key: String,
    pub stripe_api_key: String,
    pub stripe_webhook_secret: String,
    pub resend_api_key: String,
    pub resend_from_email: String,
    pub base_url: String,
    pub db_path: String,
    pub static_dir: String,
    pub public_dir: String,
    pub templates_dir: String,
    pub submissions_from_email: String,
    pub submissions_to_email: String,
}

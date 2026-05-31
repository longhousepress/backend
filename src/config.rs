use figment::value::{Dict, Map};
use figment::{Error, Metadata, Profile, Provider};
use rocket::serde::Deserialize;

pub struct SystemdCreds;

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

use ::tera::Tera;
use anyhow::Result;

use crate::config::Config;

pub const PURCHASE_EMAIL: &str = "purchase_email.html.tera";
pub const SUBMISSION_EMAIL: &str = "submission_email.html.tera";

const REQUIRED: &[&str] = &[PURCHASE_EMAIL, SUBMISSION_EMAIL];

pub fn load_tera(config: &Config) -> Result<Tera> {
    let tera = Tera::new(&format!("{}/**/*.html.tera", config.templates_dir))?;
    for name in REQUIRED {
        tera.get_template(name)
            .map_err(|_| anyhow::anyhow!("Missing required template: {name}"))?;
    }
    Ok(tera)
}

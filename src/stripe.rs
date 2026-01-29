use anyhow::Result;
use serde::{Deserialize, Serialize};
use rocket::State;
use sqlx::sqlite::SqlitePool;

use crate::models::{CheckoutRequest, CheckoutSession};

const STRIPE_KEY: &str = "REDACTED_STRIPE_KEY";

#[derive(Debug, Serialize, Deserialize)]
struct StripeCheckout {
    mode: CheckoutMode,
    success_url: String,
    cancel_url: String,
    line_items: Vec<StripeLineItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StripeLineItem {
    pub price_data: StripePriceData,
    pub quantity: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StripePriceData {
    pub currency: Currency,
    pub product_data: StripeProductData,
    pub unit_amount: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StripeProductData {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Currency {
    GBP,
    // extend as needed
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckoutMode {
    Payment,
    // extend as needed
}

pub async fn get_edition_name(id: &str, db: &SqlitePool) -> Result<String> {
    // Parse the provided id (string) into an i64 for SQLite.
    // If parsing fails, return an error. If the row is not found, return an error as well.
    let id_i64: i64 = id.parse().map_err(|_| anyhow::anyhow!("invalid edition id: {}", id))?;
    let title_opt = sqlx::query_scalar::<_, String>("SELECT title FROM editions WHERE id = ?")
        .bind(id_i64)
        .fetch_optional(db)
        .await?;
    match title_opt {
        Some(title) => Ok(title),
        None => Err(anyhow::anyhow!("edition id {} not found", id_i64)),
    }
}

pub async fn get_edition_price(id: &str, db: &SqlitePool) -> Result<u32> {
    // Parse the provided id (string) into an i64 for SQLite.
    // If parsing fails or the row is not found, return an error.
    let id_i64: i64 = id.parse().map_err(|_| anyhow::anyhow!("invalid edition id: {}", id))?;
    let price_opt = sqlx::query_scalar::<_, i64>("SELECT price FROM editions WHERE id = ?")
        .bind(id_i64)
        .fetch_optional(db)
        .await?;
    match price_opt {
        Some(price) => Ok(price as u32),
        None => Err(anyhow::anyhow!("edition id {} not found", id_i64)),
    }
}

pub async fn create_checkout_body(db: &SqlitePool, req: &CheckoutRequest) -> Result<Vec<StripeLineItem>> {
	let mut items: Vec<StripeLineItem> = Vec::with_capacity(req.items.len());
	for item in &req.items {
		let name = get_edition_name(&item.edition_id, db).await?;
		let unit_amount = get_edition_price(&item.edition_id, db).await?;
		let final_item = StripeLineItem {
			quantity: item.quantity,
			price_data: StripePriceData {
				currency: Currency::GBP,
				product_data: StripeProductData { name },
				unit_amount,
			},
		};
		items.push(final_item);
	}
	Ok(items)
}

pub async fn create_checkout_session(db: &State<SqlitePool>, req: &CheckoutRequest) -> Result<CheckoutSession> {
    // Example typed checkout session (keep the typed structs/enums in your code base)
    let example_checkout_session = StripeCheckout {
        mode: CheckoutMode::Payment,
        success_url: "http://localhost:4321/success".to_string(),
        cancel_url: "http://localhost:4321/failure".to_string(),
        line_items: create_checkout_body(db.inner(), req).await?,
    };

    // Serialize the typed struct into a nested querystring using serde_qs
    // serde_qs respects serde attributes such as #[serde(rename_all = "lowercase")]
    let encoded = serde_qs::to_string(&example_checkout_session)?;

    // Send to Stripe
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.stripe.com/v1/checkout/sessions")
        .header("Authorization", format!("Bearer {}", STRIPE_KEY))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(encoded)
        .send()
        .await?
        .text()
        .await?;


    let deserialized_response: CheckoutSession = serde_json::from_str(&response)?;

    Ok(deserialized_response)
}

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;
use rocket::{serde::json::Json, State, http::Status};
use tracing::{error, warn};

use crate::config::Config;
use crate::db::{get_edition_name, get_edition_price};


#[post("/api/checkout", data = "<request>")]
pub async fn checkout(config: &State<Config>, db: &State<SqlitePool>, request: Json<CheckoutRequest>) -> Result<Json<CheckoutSession>, Status> {
    // take ownership of the parsed request body
    let req = request.into_inner();
    match create_checkout_session(config, &db, &req).await {
        Ok(s) => Ok(Json(s)),
        Err(e) => {
        	error!("Error creating checkout session: {}", e);
        	Err(Status::InternalServerError)
        }
    }
}

pub async fn create_checkout_session(config: &State<Config>, db: &State<SqlitePool>, req: &CheckoutRequest) -> Result<CheckoutSession> {
    // Persist a pending order in the DB and get its number
    let checkout = StripeCheckout {
        mode: CheckoutMode::Payment,
        success_url: format!("http://localhost:4321/success?session_id={{CHECKOUT_SESSION_ID}}"),
        cancel_url: "http://localhost:4321/failure".into(),
        line_items: create_checkout_body(db.inner(), req).await?,
        client_reference_id: None,
        payment_intent_data: None,
    };

    // Serialize the typed struct into a nested querystring structure that Stripe expects
    let encoded = serde_qs::to_string(&checkout)?;

    // Send to Stripe
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.stripe.com/v1/checkout/sessions")
        .header("Authorization", format!("Bearer {}", config.stripe_api_key))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(encoded)
        .send()
        .await?;

    // Check that we didn't get an error (either client or server)
    if response.status().is_client_error() || response.status().is_server_error() {
        return Err(anyhow::anyhow!("stripe returned {}: {}", response.status(), response.text().await?));
    }

    // Get the text of the successful response
    let response_text = response
    	.text()
     	.await?;

    // Parse Stripe response to extract session id and url
    let stripe_json: StripeCheckoutSessionResponse = serde_json::from_str(&response_text)?;

    let stripe_session_id = stripe_json.id;
    let url = stripe_json.url;

    // Update our order row with the stripe_session_id
    match req.persist(db.inner(), &stripe_session_id, Some("GBP")).await {
    	Ok(_) => Ok(CheckoutSession { url }),
     	// If the DB insert fails, we need to clean up the dangling session
    	Err(e) => {
     		error!("Failed to persist order for Stripe session {}: {}", stripe_session_id, e);
     		if let Err(expire_err) = expire_stripe_session(config, &stripe_session_id).await {
     			warn!("Failed to expire dangling Stripe session {}: {}", stripe_session_id, expire_err);
     		}
       		Err(e)
     	}
    }
}

async fn expire_stripe_session(config: &Config, id: &str) -> Result<()> {
 // Send to Stripe
    let client = reqwest::Client::new();
    client
        .post(format!("https://api.stripe.com/v1/checkout/sessions/{id}/expire"))
        .header("Authorization", format!("Bearer {}", config.stripe_api_key))
        .send()
        .await?;

	Ok(())
}

pub async fn create_checkout_body(db: &SqlitePool, req: &CheckoutRequest) -> Result<Vec<StripeLineItem>> {
	let mut items: Vec<StripeLineItem> = Vec::with_capacity(req.items.len());
	for item in &req.items {
		let name = get_edition_name(item.edition_id, db).await?;
		let unit_amount = get_edition_price(item.edition_id, db).await?;
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

#[derive(Serialize, Deserialize)]
struct StripeCheckoutSessionResponse {
	id: String,
 	url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StripeCheckout {
    mode: CheckoutMode,
    success_url: String,
    cancel_url: String,
    line_items: Vec<StripeLineItem>,
    // Optional client_reference_id so we can attach our internal order_id to the Stripe session
    client_reference_id: Option<String>,
    // Optional payment_intent_data allows attaching metadata to the PaymentIntent created by Stripe
    payment_intent_data: Option<PaymentIntentData>,
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
pub struct PaymentIntentData {
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Currency {
    GBP,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckoutMode {
    Payment,
}

// What the front end POSTs to us
#[derive(Serialize, Deserialize, FromRow)]
pub struct CheckoutRequest {
    pub items: Vec<CheckoutItem>,
}

// What we will return to the front end
#[derive(Serialize, Deserialize, FromRow)]
pub struct CheckoutSession {
    pub url: String,
}


#[derive(Serialize, Deserialize, FromRow)]
pub struct CheckoutItem {
    pub edition_id: i64,
    pub quantity: u32,
}

#[allow(dead_code)]
impl CheckoutRequest {
    /// Persist this checkout request as an `orders` row and associated `order_items`.
    ///
    /// Behavior:
    /// - Looks up each edition's current `price` and sums the total.
    /// - Inserts a row into `orders` (with optional `client_reference` and `stripe_session_id`).
    /// - Inserts one `order_items` row per `CheckoutItem`, recording `price_at_purchase`.
    /// - All work is performed inside a sqlx transaction to ensure atomicity: either the order
    ///   and all its order_items are inserted, or nothing is committed.
    pub async fn persist(
        &self,
        pool: &SqlitePool,
        stripe_session_id: &str,
        currency: Option<&str>,
    ) -> Result<i64> {
        // Start a transaction so all operations are atomic
        let mut tx = pool.begin().await?;

        // Compute total and validate edition ids within the transaction
        let mut total_amount: i64 = 0;
        for item in &self.items {
            let edition_id: i64 = item.edition_id;

            // Read the current price for this edition using the transaction
            let row = sqlx::query!("SELECT price FROM editions WHERE id = ?", edition_id)
                .fetch_one(&mut *tx)
                .await?;
            let price: i64 = row.price;
            total_amount += price * (item.quantity as i64);
        }

        // Insert the order (paid is NULL for pending) inside the transaction
        let res = sqlx::query(
            "INSERT INTO orders (stripe_session_id, paid, total_amount, currency) VALUES (?, NULL, ?, ?)",
        )
        .bind(stripe_session_id)
        .bind(total_amount)
        .bind(currency)
        .execute(&mut *tx)
        .await?;

        let order_id = res.last_insert_rowid();

        // Insert order_items (capture price_at_purchase inside the same transaction)
        for item in &self.items {
            let edition_id: i64 = item.edition_id;

            // capture the price at purchase time again to ensure consistency
            let row = sqlx::query!("SELECT price FROM editions WHERE id = ?", edition_id)
                .fetch_one(&mut *tx)
                .await?;
            let price_at_purchase: i64 = row.price;

            sqlx::query(
                "INSERT INTO order_items (order_id, edition_id, quantity, price_at_purchase) VALUES (?, ?, ?, ?)",
            )
            .bind(order_id)
            .bind(edition_id)
            .bind(item.quantity as i64)
            .bind(price_at_purchase)
            .execute(&mut *tx)
            .await?;
        }

        // Commit the transaction; if commit fails the transaction will be rolled back.
        tx.commit().await?;

        Ok(order_id)
    }
}

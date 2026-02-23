use anyhow::Result;
use email_address::EmailAddress;
use rocket::{State, http::Status, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

use crate::config::Config;
use crate::db::{get_edition_name, get_edition_price};

#[post("/checkout", data = "<request>")]
pub async fn checkout(
    config: &State<Config>,
    db: &State<SqlitePool>,
    request: Json<CheckoutRequest>,
) -> Result<Json<CheckoutSession>, Status> {
    // take ownership of the parsed request body
    let req = request.into_inner();
    
    // Validate the request before processing
    if let Err(e) = validate_checkout_request(&req, db).await {
        rocket::warn!("Invalid checkout request: {}", e);
        return Err(Status::BadRequest);
    }
    
    match create_checkout_session(config, db, &req).await {
        Ok(s) => Ok(Json(s)),
        Err(e) => {
            rocket::error!("Error creating checkout session: {}", e);
            Err(Status::InternalServerError)
        }
    }
}

async fn validate_checkout_request(req: &CheckoutRequest, db: &SqlitePool) -> Result<()> {
    // Validate email format
    if !EmailAddress::is_valid(&req.email) {
        return Err(anyhow::anyhow!("Invalid email address"));
    }
    
    // Validate items exist and are not empty
    if req.items.is_empty() {
        return Err(anyhow::anyhow!("Checkout must contain at least one item"));
    }
    
    // Limit total number of line items (prevent DoS)
    if req.items.len() > 50 {
        return Err(anyhow::anyhow!("Checkout cannot contain more than 50 items"));
    }
    
    // Validate each item
    let currency_str = req.currency.as_str();
    for item in &req.items {
        // Validate quantity is not zero and not too high
        if item.quantity == 0 {
            return Err(anyhow::anyhow!("Item quantity must be at least 1"));
        }
        if item.quantity > 100 {
            return Err(anyhow::anyhow!("Item quantity cannot exceed 100"));
        }
        
        // Check that the edition exists, is listed, and has a price for the requested currency
        let result = sqlx::query!(
            "SELECT e.listed, ep.price 
             FROM editions e
             LEFT JOIN edition_prices ep ON e.id = ep.edition_id AND ep.currency = ?
             WHERE e.id = ?",
            currency_str,
            item.edition_id
        )
        .fetch_optional(db)
        .await?;
        
        match result {
            None => return Err(anyhow::anyhow!("Edition {} not found", item.edition_id)),
            Some(row) => {
                if row.listed != Some(1) {
                    return Err(anyhow::anyhow!("Edition {} is not available for purchase", item.edition_id));
                }
                if row.price.is_none() {
                    return Err(anyhow::anyhow!("Edition {} does not have a price for currency {}", 
                        item.edition_id, currency_str));
                }
            }
        }
    }
    
    Ok(())
}

pub async fn create_checkout_session(
    config: &State<Config>,
    db: &State<SqlitePool>,
    req: &CheckoutRequest,
) -> Result<CheckoutSession> {
    // Persist a pending order in the DB and get its number
    let checkout = StripeCheckout {
        mode: CheckoutMode::Payment,
        success_url: config.stripe_success_url.clone(),
        cancel_url: config.stripe_cancel_url.clone(),
        line_items: create_checkout_body(db.inner(), req, &req.currency).await?,
        customer_email: Some(req.email.clone()),
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
        return Err(anyhow::anyhow!(
            "stripe returned {}: {}",
            response.status(),
            response.text().await?
        ));
    }

    // Get the text of the successful response
    let response_text = response.text().await?;

    // Parse Stripe response to extract session id and url
    let stripe_json: StripeCheckoutSessionResponse = serde_json::from_str(&response_text)?;

    let stripe_session_id = stripe_json.id;
    let url = stripe_json.url;

    // Update our order row with the stripe_session_id
    match req
        .persist(db.inner(), &stripe_session_id, Some(req.currency.as_str()))
        .await
    {
        Ok(_) => Ok(CheckoutSession { url }),
        // If the DB insert fails, we need to clean up the dangling session
        Err(e) => {
            rocket::error!(
                "Failed to persist order for Stripe session {}: {}",
                stripe_session_id,
                e
            );
            if let Err(expire_err) = expire_stripe_session(config, &stripe_session_id).await {
                rocket::warn!(
                    "Failed to expire dangling Stripe session {}: {}",
                    stripe_session_id,
                    expire_err
                );
            }
            Err(e)
        }
    }
}

async fn expire_stripe_session(config: &Config, id: &str) -> Result<()> {
    // Send to Stripe
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "https://api.stripe.com/v1/checkout/sessions/{id}/expire"
        ))
        .header("Authorization", format!("Bearer {}", config.stripe_api_key))
        .send()
        .await?;

    // Check response status
    if response.status().is_client_error() || response.status().is_server_error() {
        return Err(anyhow::anyhow!(
            "Failed to expire Stripe session {}: status {}",
            id,
            response.status()
        ));
    }

    Ok(())
}

pub async fn create_checkout_body(
    db: &SqlitePool,
    req: &CheckoutRequest,
    currency: &Currency,
) -> Result<Vec<StripeLineItem>> {
    let mut items: Vec<StripeLineItem> = Vec::with_capacity(req.items.len());
    for item in &req.items {
        let name = get_edition_name(item.edition_id, db).await?;
        let unit_amount = get_edition_price(item.edition_id, currency.as_str(), db).await?;
        
        // Check for potential overflow when calculating line item total
        let quantity_u64 = item.quantity as u64;
        let unit_amount_u64 = unit_amount as u64;
        quantity_u64.checked_mul(unit_amount_u64)
            .ok_or_else(|| anyhow::anyhow!("Price calculation overflow for edition {}", item.edition_id))?;
        
        let final_item = StripeLineItem {
            quantity: item.quantity,
            price_data: StripePriceData {
                currency: currency.clone(),
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
    // Optional customer_email to pre-fill the email field in Stripe checkout
    customer_email: Option<String>,
    // Optional client_reference_id so we can attach our internal order_id to the Stripe session
    client_reference_id: Option<String>,
    // Optional payment_intent_data allows attaching metadata to the PaymentIntent created by Stripe
    payment_intent_data: Option<PaymentIntentData>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StripeLineItem {
    pub price_data: StripePriceData,
    pub quantity: u8,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
}

impl Currency {
    // Convert Currency enum to uppercase string for database queries and Stripe API
    pub fn as_str(&self) -> &str {
        match self {
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
            Currency::Gbp => "GBP",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckoutMode {
    Payment,
}

// What the front end POSTs to us
#[derive(Serialize, Deserialize, FromRow)]
pub struct CheckoutRequest {
    pub email: String,
    pub currency: Currency,
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
    pub quantity: u8,
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
        let currency = currency.unwrap_or("GBP");
        let mut total_amount: i64 = 0;
        for item in &self.items {
            let edition_id: i64 = item.edition_id;

            // Read the current price for this edition using the transaction
            let row = sqlx::query!(
                "SELECT price FROM edition_prices WHERE edition_id = ? AND currency = ?",
                edition_id,
                currency
            )
            .fetch_one(&mut *tx)
            .await?;
            let price: i64 = row.price;
            
            // Use checked multiplication to prevent overflow
            let line_total = price.checked_mul(item.quantity as i64)
                .ok_or_else(|| anyhow::anyhow!("Price overflow for edition {}", edition_id))?;
            
            // Use checked addition to prevent overflow
            total_amount = total_amount.checked_add(line_total)
                .ok_or_else(|| anyhow::anyhow!("Total amount overflow"))?;
        }

        // Insert the order (paid is NULL for pending) inside the transaction
        let res = sqlx::query(
            "INSERT INTO orders (stripe_session_id, email, paid, total_amount, currency) VALUES (?, ?, NULL, ?, ?)",
        )
        .bind(stripe_session_id)
        .bind(&self.email)
        .bind(total_amount)
        .bind(currency)
        .execute(&mut *tx)
        .await?;

        let order_id = res.last_insert_rowid();

        // Insert order_items (capture price_at_purchase inside the same transaction)
        for item in &self.items {
            let edition_id: i64 = item.edition_id;

            // capture the price at purchase time again to ensure consistency
            let row = sqlx::query!(
                "SELECT price FROM edition_prices WHERE edition_id = ? AND currency = ?",
                edition_id,
                currency
            )
            .fetch_one(&mut *tx)
            .await?;
            let price_at_purchase: i64 = row.price;

            sqlx::query(
                "INSERT INTO order_items (order_id, edition_id, quantity, price_at_purchase, currency_at_purchase) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(order_id)
            .bind(edition_id)
            .bind(item.quantity as i64)
            .bind(price_at_purchase)
            .bind(currency)
            .execute(&mut *tx)
            .await?;
        }

        // Commit the transaction; if commit fails the transaction will be rolled back.
        tx.commit().await?;

        Ok(order_id)
    }
}

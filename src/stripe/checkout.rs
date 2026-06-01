use anyhow::Result;
use email_address::EmailAddress;
use reqwest::Client;
use rocket::{State, http::Status, serde::json::Json};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;

use crate::config::Config;
use crate::db::{check_files_exist, create_order};
use crate::models::Currency;

#[post("/checkout", data = "<request>")]
pub async fn checkout(
    config: &State<Config>,
    db: &State<SqlitePool>,
    http_client: &State<Client>,
    request: Json<CheckoutRequest>,
) -> Result<Json<CheckoutSession>, Status> {
    let req = request.into_inner();

    let resolved = match validate_checkout_request(&req, db, &config.static_dir).await {
        Ok(r) => r,
        Err(e) => {
            rocket::warn!("Invalid checkout request: {}", e);
            return Err(Status::BadRequest);
        }
    };

    match create_checkout_session(config, db, http_client, &req, resolved).await {
        Ok(s) => Ok(Json(s)),
        Err(e) => {
            rocket::error!("Error creating checkout session: {}", e);
            Err(Status::InternalServerError)
        }
    }
}

pub struct ResolvedCheckoutItem {
    pub edition_id: i64,
    pub name: String,
    pub unit_amount: u32,
    pub quantity: u8,
}

async fn validate_checkout_request(req: &CheckoutRequest, db: &SqlitePool, static_dir: &str) -> Result<Vec<ResolvedCheckoutItem>> {
    if !EmailAddress::is_valid(&req.email) {
        return Err(anyhow::anyhow!("Invalid email address"));
    }

    if req.items.is_empty() {
        return Err(anyhow::anyhow!("Checkout must contain at least one item"));
    }

    if req.items.len() > 50 {
        return Err(anyhow::anyhow!(
            "Checkout cannot contain more than 50 items"
        ));
    }

    let currency_str = req.currency.as_str();
    let mut resolved = Vec::with_capacity(req.items.len());

    for item in &req.items {
        if item.quantity == 0 {
            return Err(anyhow::anyhow!("Item quantity must be at least 1"));
        }
        if item.quantity > 100 {
            return Err(anyhow::anyhow!("Item quantity cannot exceed 100"));
        }

        let result = sqlx::query!(
            "SELECT e.listed, ep.price, bl.title
             FROM editions e
             LEFT JOIN edition_prices ep ON e.id = ep.edition_id AND ep.currency = ?
             INNER JOIN book_localizations bl ON bl.book_id = e.book_id AND bl.language = e.language
             WHERE e.id = ?",
            currency_str,
            item.edition_id
        )
        .fetch_optional(db)
        .await?;

        let (title, price) = match result {
            None => return Err(anyhow::anyhow!("Edition {} not found", item.edition_id)),
            Some(row) => {
                if row.listed != Some(1) {
                    return Err(anyhow::anyhow!(
                        "Edition {} is not available for purchase",
                        item.edition_id
                    ));
                }
                let price = row.price.ok_or_else(|| anyhow::anyhow!(
                    "Edition {} does not have a price for currency {}",
                    item.edition_id,
                    currency_str
                ))?;
                (row.title, price)
            }
        };

        if !check_files_exist(item.edition_id, static_dir, db).await? {
            return Err(anyhow::anyhow!(
                "Edition {} is not fulfillable: one or more files are missing",
                item.edition_id
            ));
        }

        resolved.push(ResolvedCheckoutItem {
            edition_id: item.edition_id,
            name: title,
            unit_amount: price as u32,
            quantity: item.quantity,
        });
    }

    Ok(resolved)
}

pub async fn create_checkout_session(
    config: &State<Config>,
    db: &State<SqlitePool>,
    http_client: &State<Client>,
    req: &CheckoutRequest,
    resolved: Vec<ResolvedCheckoutItem>,
) -> Result<CheckoutSession> {
    let checkout = StripeCheckout {
        mode: CheckoutMode::Payment,
        success_url: format!("{}/success?session_id={{CHECKOUT_SESSION_ID}}", config.base_url),
        cancel_url: format!("{}/failure", config.base_url),
        line_items: create_checkout_body(&resolved, &req.currency)?,
        customer_email: Some(req.email.clone()),
        client_reference_id: None,
        payment_intent_data: None,
    };

    // Serialize the typed struct into a nested querystring structure that Stripe expects
    let encoded = serde_qs::to_string(&checkout)?;

    // Send to Stripe
    let response = http_client
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
    let items: Vec<(i64, u8)> = req.items.iter().map(|i| (i.edition_id, i.quantity)).collect();
    match create_order(db.inner(), &req.email, &stripe_session_id, req.currency.as_str(), &items).await
    {
        Ok(_) => Ok(CheckoutSession { url }),
        // If the DB insert fails, we need to clean up the dangling session
        Err(e) => {
            rocket::error!(
                "Failed to persist order for Stripe session {}: {}",
                stripe_session_id,
                e
            );
            if let Err(expire_err) = expire_stripe_session(config, &stripe_session_id, http_client).await {
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

async fn expire_stripe_session(config: &Config, id: &str, client: &Client) -> Result<()> {
    // Send to Stripe
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

pub fn create_checkout_body(
    resolved: &[ResolvedCheckoutItem],
    currency: &Currency,
) -> Result<Vec<StripeLineItem>> {
    let mut items: Vec<StripeLineItem> = Vec::with_capacity(resolved.len());
    for item in resolved {
        let quantity_u64 = item.quantity as u64;
        let unit_amount_u64 = item.unit_amount as u64;
        quantity_u64.checked_mul(unit_amount_u64).ok_or_else(|| {
            anyhow::anyhow!("Price calculation overflow for edition {}", item.edition_id)
        })?;

        items.push(StripeLineItem {
            quantity: item.quantity,
            price_data: StripePriceData {
                currency: currency.clone(),
                product_data: StripeProductData { name: item.name.clone() },
                unit_amount: item.unit_amount,
            },
        });
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckoutMode {
    Payment,
}

// What the front end POSTs to us
#[derive(Serialize, Deserialize)]
pub struct CheckoutRequest {
    pub email: String,
    pub currency: Currency,
    pub items: Vec<CheckoutItem>,
}

// What we will return to the front end
#[derive(Serialize, Deserialize)]
pub struct CheckoutSession {
    pub url: String,
}

#[derive(Serialize, Deserialize)]
pub struct CheckoutItem {
    pub edition_id: i64,
    pub quantity: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(edition_id: i64, name: &str, unit_amount: u32, quantity: u8) -> ResolvedCheckoutItem {
        ResolvedCheckoutItem {
            edition_id,
            name: name.to_string(),
            unit_amount,
            quantity,
        }
    }

    #[test]
    fn single_item_produces_correct_line_item() {
        let items = vec![make_item(1, "Test Book", 1500, 1)];
        let result = create_checkout_body(&items, &Currency::Usd).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].quantity, 1);
        assert_eq!(result[0].price_data.unit_amount, 1500);
        assert_eq!(result[0].price_data.product_data.name, "Test Book");
    }

    #[test]
    fn multiple_items_produce_correct_line_items() {
        let items = vec![
            make_item(1, "Book A", 1000, 2),
            make_item(2, "Book B", 2500, 1),
            make_item(3, "Book C", 500, 3),
        ];
        let result = create_checkout_body(&items, &Currency::Eur).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].price_data.product_data.name, "Book A");
        assert_eq!(result[0].quantity, 2);
        assert_eq!(result[1].price_data.product_data.name, "Book B");
        assert_eq!(result[1].quantity, 1);
        assert_eq!(result[2].price_data.product_data.name, "Book C");
        assert_eq!(result[2].quantity, 3);
    }

    #[test]
    fn empty_items_returns_empty() {
        let items: Vec<ResolvedCheckoutItem> = vec![];
        let result = create_checkout_body(&items, &Currency::Usd).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn currency_is_preserved() {
        let items = vec![make_item(1, "Book", 1000, 1)];
        let result = create_checkout_body(&items, &Currency::Krw).unwrap();
        // Currency should be Krw for all items
        let serialized = serde_json::to_string(&result[0].price_data.currency).unwrap();
        assert_eq!(serialized, "\"KRW\"");
    }

    #[test]
    fn max_quantity_times_max_amount_does_not_overflow() {
        // u8::MAX (255) * u32::MAX would overflow u64, but u8::MAX * a reasonable amount shouldn't
        let items = vec![make_item(1, "Expensive", 100_000, 255)]; // 255 * 100000 = 25,500,000
        let result = create_checkout_body(&items, &Currency::Usd);
        assert!(result.is_ok());
    }
}

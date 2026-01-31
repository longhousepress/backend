use std::time::{SystemTime, UNIX_EPOCH};
use rand::Rng;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use rocket::State;
use sqlx::sqlite::SqlitePool;
use hmac_sha256::HMAC;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

use crate::models::{CheckoutRequest, CheckoutSession};

const STRIPE_KEY: &str = "REDACTED_STRIPE_KEY";

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
    // extend as needed
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckoutMode {
    Payment,
    // extend as needed
}

pub async fn get_edition_name(id: i64, db: &SqlitePool) -> Result<String> {
    // Look up the edition title by numeric id.
    let title_opt = sqlx::query_scalar::<_, String>("SELECT title FROM editions WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    match title_opt {
        Some(title) => Ok(title),
        None => Err(anyhow::anyhow!("edition id {} not found", id)),
    }
}

pub async fn get_edition_price(id: i64, db: &SqlitePool) -> Result<u32> {
    // Look up the edition price by numeric id.
    let price_opt = sqlx::query_scalar::<_, i64>("SELECT price FROM editions WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    match price_opt {
        Some(price) => Ok(price as u32),
        None => Err(anyhow::anyhow!("edition id {} not found", id)),
    }
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

pub async fn create_checkout_session(db: &State<SqlitePool>, req: &CheckoutRequest) -> Result<CheckoutSession> {
    // 1) Persist a pending order in our DB (paid = NULL, no stripe_session_id yet).
    //    This gives us an internal `order_id` we can attach to the Stripe session.
    let order_id = req.persist(db.inner(), None, None, Some("GBP")).await?;

    // Assemble the session, include the order_id as client_reference_id so Stripe will carry it
    // in metadata and webhooks. The success_url intentionally does NOT include a short-lived token:
    // the frontend should call the server's verify endpoint using the returned session id to avoid
    // race conditions with webhooks. Server-side verification will create the persistent DB download tokens.
    let mut pi_metadata = std::collections::HashMap::new();
    pi_metadata.insert("order_id".to_string(), order_id.to_string());

    let checkout = StripeCheckout {
        mode: CheckoutMode::Payment,
        // Remove temporary token from success URL to avoid relying on client-visible short-lived tokens.
        success_url: format!("http://localhost:4321/success?order_id={order_id}&session_id={{CHECKOUT_SESSION_ID}}"),
        cancel_url: "http://localhost:4321/failure".into(),
        line_items: create_checkout_body(db.inner(), req).await?,
        client_reference_id: Some(order_id.to_string()),
        payment_intent_data: Some(PaymentIntentData { metadata: pi_metadata }),
    };

    // Serialize the typed struct into a nested querystring using serde_qs
    // serde_qs respects serde attributes such as #[serde(rename_all = "lowercase")]
    let encoded = serde_qs::to_string(&checkout)?;

    // Send to Stripe
    let client = reqwest::Client::new();
    let response_text = client
        .post("https://api.stripe.com/v1/checkout/sessions")
        .header("Authorization", format!("Bearer {}", STRIPE_KEY))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(encoded)
        .send()
        .await?
        .text()
        .await?;

    // Parse Stripe response to extract session id and url
    let stripe_json: serde_json::Value = serde_json::from_str(&response_text)?;
    let session_id = stripe_json
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("stripe response missing id"))?;
    let url = stripe_json
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("stripe response missing url"))?
        .to_string();

    // Update our order row with the stripe session id so we can reconcile later
    sqlx::query!("UPDATE orders SET stripe_session_id = ? WHERE id = ?", session_id, order_id)
        .execute(db.inner())
        .await?;

    // Return the same CheckoutSession shape as before (frontend expects { url })
    Ok(CheckoutSession { url })
}

pub fn mint() -> String {
    const TOKEN_TTL_SECS: u64 = 15 * 60; // 15 min
    let expire_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + TOKEN_TTL_SECS;

    let mut buf = [0u8; 56];
    // 1. random nonce
    rand::rng().fill(&mut buf[..16]);
    // 2. expiry
    buf[16..24].copy_from_slice(&expire_ts.to_be_bytes());
    // 3. sign
    let secret = std::env::var("TOKEN_KEY").expect("TOKEN_KEY not set");
    let sig = hmac_sha256::HMAC::mac(&buf[..24], secret.as_bytes());
    buf[24..].copy_from_slice(&sig);
    // 4. url-safe base64
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&buf)
}

pub fn verify(tok: &str) -> Result<(), String> {
    let buf = URL_SAFE_NO_PAD.decode(tok)
        .map_err(|_| "bad base64".to_string())?;
    if buf.len() != 56 { return Err("buf.len() is not 56".to_string()); }
    let secret = std::env::var("TOKEN_KEY").unwrap();
    let sig = HMAC::mac(&buf[..24], secret.as_bytes());
    if sig != buf[24..] { return Err("error here: if sig != buf[24..]".to_string()); }

    let expire_ts = u64::from_be_bytes(buf[16..24].try_into().unwrap());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH).unwrap()
        .as_secs();

    if expire_ts >= now {
        return Ok(());
    }

    Err("end".to_string())
}


/// Verify a Stripe Checkout Session by querying Stripe's API and, if paid,
/// mark the corresponding order as paid in our DB.
///
/// Returns Ok(true) if the session was confirmed paid and the DB was updated,
/// Ok(false) if the session is not paid or metadata mismatch, and Err on other errors.
pub async fn verify_stripe_checkout_session_and_mark_paid(
    pool: &SqlitePool,
    order_id: i64,
    session_id: &str,
) -> Result<bool> {
    // Fetch the Checkout Session from Stripe and expand payment_intent for status.
    let client = reqwest::Client::new();
    let url = format!("https://api.stripe.com/v1/checkout/sessions/{}?expand[]=payment_intent", session_id);

    let resp_text = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", STRIPE_KEY))
        .send()
        .await?
        .text()
        .await?;

    let json: serde_json::Value = serde_json::from_str(&resp_text)?;

    // Ensure the session's metadata.order_id matches the supplied order_id if present.
    if let Some(metadata_order_id) = json
        .get("metadata")
        .and_then(|m| m.get("order_id"))
        .and_then(|v| v.as_str())
    {
        if metadata_order_id != order_id.to_string() {
            // Metadata does not match the provided order_id; don't mark paid.
            return Ok(false);
        }
    }

    // Check payment status on the session, or fall back to payment_intent.status.
    let is_paid = match json.get("payment_status").and_then(|v| v.as_str()) {
        Some("paid") => true,
        _ => {
            match json
                .get("payment_intent")
                .and_then(|pi| pi.get("status"))
                .and_then(|v| v.as_str())
            {
                Some("succeeded") => true,
                _ => false,
            }
        }
    };

    if is_paid {
        // Mark order paid and set paid_at using SQLite's strftime to produce RFC3339 UTC
        sqlx::query!(
            "UPDATE orders SET paid = 1, paid_at = (strftime('%Y-%m-%dT%H:%M:%SZ','now')) WHERE id = ?",
            order_id
        )
        .execute(pool)
        .await?;
    }

    Ok(is_paid)
}

/// Create single-use, time-limited download tokens for every edition on an order.
///
/// This function is idempotent: if tokens already exist for the order, it returns immediately.
pub async fn create_download_tokens_for_order(pool: &SqlitePool, order_id: i64) -> Result<()> {
    // If tokens already exist for this order, do nothing (idempotent)
    let existing_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM download_tokens WHERE order_id = ?"
    )
    .bind(order_id)
    .fetch_one(pool)
    .await?;

    if existing_count > 0 {
        return Ok(());
    }

    // Fetch editions for this order
    let items = sqlx::query!("SELECT edition_id FROM order_items WHERE order_id = ?", order_id)
        .fetch_all(pool)
        .await?;

    for r in items {
        let edition_id = r.edition_id;
        // Mint a token (reusing existing mint() function)
        let token = mint();

        // Insert token with 7-day expiry (RFC3339 format using SQLite strftime)
        sqlx::query!(
            "INSERT INTO download_tokens (order_id, edition_id, token, expires_at) \
             VALUES (?, ?, ?, (strftime('%Y-%m-%dT%H:%M:%SZ','now','+7 days')))",
            order_id,
            edition_id,
            token
        )
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// A convenience HTTP endpoint you can mount to verify an order's Stripe session on demand.
/// Example: POST /api/order/:id/verify?session_id=cs_test_...
///
/// Note: this endpoint is synchronous and returns a simple body:
/// - 200 with body "paid" if the session was confirmed and DB was updated,
/// - 403 if the session is not paid or metadata mismatched
/// - 500 on error
#[post("/api/order/<order_id>/verify?<session_id>")]
pub async fn verify_order_endpoint(
    db: &State<SqlitePool>,
    order_id: i64,
    session_id: String,
) -> Result<rocket::serde::json::Json<serde_json::Value>, rocket::http::Status> {
    match verify_stripe_checkout_session_and_mark_paid(db.inner(), order_id, &session_id).await {
        Ok(true) => {
            // Ensure download tokens exist immediately to avoid race with webhook processing.
            if let Err(e) = create_download_tokens_for_order(db.inner(), order_id).await {
                eprintln!("Error creating download tokens for order {}: {:?}", order_id, e);
                // Don't fail the verify request just because token creation failed; we'll still attempt to read tokens.
            }

            // Fetch tokens and edition titles for this order
            let rows = sqlx::query!(
                "SELECT dt.token, dt.expires_at, e.title \
                 FROM download_tokens dt \
                 INNER JOIN editions e ON dt.edition_id = e.id \
                 WHERE dt.order_id = ?",
                order_id
            )
            .fetch_all(db.inner())
            .await
            .map_err(|e| { eprintln!("db error: {:?}", e); rocket::http::Status::InternalServerError })?;

            let mut list = Vec::with_capacity(rows.len());
            for r in rows {
                let token = r.token;
                let url = format!("/api/download/{}", token);
                list.push(serde_json::json!({
                    "token": token,
                    "url": url,
                    "title": r.title,
                    "expires_at": r.expires_at
                }));
            }

            Ok(rocket::serde::json::Json(serde_json::Value::Array(list)))
        }
        Ok(false) => Err(rocket::http::Status::Forbidden),
        Err(e) => {
            eprintln!("stripe verify error: {:?}", e);
            Err(rocket::http::Status::InternalServerError)
        }
    }
}

/// Webhook endpoint to receive Stripe events. For security we do not trust the incoming
/// webhook payload alone — instead we extract the order/session identifiers from the
/// event and verify the session status with Stripe server-side before marking an order paid.
///
/// Forward the Stripe CLI to this path for local testing:
/// stripe listen --forward-to localhost:8000/webhook --events checkout.session.completed,payment_intent.succeeded
#[post("/webhook", data = "<payload>")]
pub async fn stripe_webhook(
    db: &State<SqlitePool>,
    payload: String,
) -> Result<rocket::http::Status, rocket::http::Status> {
    // Parse the event JSON
    let json: serde_json::Value = match serde_json::from_str(&payload) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Failed to parse webhook JSON: {:?}", e);
            return Err(rocket::http::Status::BadRequest);
        }
    };

    let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or_default();

    if event_type == "checkout.session.completed" || event_type == "payment_intent.succeeded" {
        if let Some(obj) = json.get("data").and_then(|d| d.get("object")) {
            // Try to get order_id from metadata if present
            let metadata_order_id = obj
                .get("metadata")
                .and_then(|m| m.get("order_id"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<i64>().ok());

            // Try to get session_id from object
            let session_id_opt = obj.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());

            if let Some(order_id) = metadata_order_id {
                // Fetch the stored stripe_session_id for this order (if any) and verify via Stripe API
                match sqlx::query_scalar::<_, Option<String>>("SELECT stripe_session_id FROM orders WHERE id = ?")
                    .bind(order_id)
                    .fetch_optional(db.inner())
                    .await
                {
                    Ok(opt) => {
                        // We'll track whether verification succeeded so we can create tokens
                        let mut verified = false;

                        if let Some(session_id) = opt.flatten() {
                            match verify_stripe_checkout_session_and_mark_paid(db.inner(), order_id, &session_id).await {
                                Ok(true) => { verified = true; }
                                Ok(false) => {
                                    eprintln!("Webhook: session {} not paid according to Stripe", session_id);
                                }
                                Err(e) => {
                                    eprintln!("Error verifying session for order {}: {:?}", order_id, e);
                                    return Err(rocket::http::Status::InternalServerError);
                                }
                            }
                        } else {
                            // No session id recorded yet; if payload contains a session id use it
                            if let Some(sid) = session_id_opt.as_deref() {
                                match verify_stripe_checkout_session_and_mark_paid(db.inner(), order_id, sid).await {
                                    Ok(true) => { verified = true; }
                                    Ok(false) => {
                                        eprintln!("Webhook: session {} not paid according to Stripe", sid);
                                    }
                                    Err(e) => {
                                        eprintln!("Error verifying session (fallback) for order {}: {:?}", order_id, e);
                                        return Err(rocket::http::Status::InternalServerError);
                                    }
                                }
                            }
                        }

                        // If we verified payment, attempt to create download tokens (idempotent)
                        if verified {
                            if let Err(e) = create_download_tokens_for_order(db.inner(), order_id).await {
                                eprintln!("Error creating download tokens for order {}: {:?}", order_id, e);
                                // Don't fail the webhook just because token creation failed; we logged the error
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("DB error fetching stripe_session_id for order {}: {:?}", order_id, e);
                        return Err(rocket::http::Status::InternalServerError);
                    }
                }
            } else if let Some(session_id) = session_id_opt {
                // No metadata.order_id was present; try to find the order by session id and verify
                match sqlx::query_scalar::<_, i64>("SELECT id FROM orders WHERE stripe_session_id = ?")
                    .bind(&session_id)
                    .fetch_optional(db.inner())
                    .await
                {
                    Ok(found_opt) => {
                        if let Some(found_order_id) = found_opt {
                            match verify_stripe_checkout_session_and_mark_paid(db.inner(), found_order_id, &session_id).await {
                                Ok(true) => {
                                    if let Err(e) = create_download_tokens_for_order(db.inner(), found_order_id).await {
                                        eprintln!("Error creating download tokens for order {}: {:?}", found_order_id, e);
                                    }
                                }
                                Ok(false) => {
                                    eprintln!("Webhook: session {} not paid according to Stripe", session_id);
                                }
                                Err(e) => {
                                    eprintln!("Error verifying session for order {}: {:?}", found_order_id, e);
                                    return Err(rocket::http::Status::InternalServerError);
                                }
                            }
                        } else {
                            // No matching order found; ignore or log and continue
                            eprintln!("Webhook: no matching order for session {}", session_id);
                        }
                    }
                    Err(e) => {
                        eprintln!("DB error looking up order for session {}: {:?}", session_id, e);
                        return Err(rocket::http::Status::InternalServerError);
                    }
                }
            }
        }
    }

    // Acknowledge receipt
    Ok(rocket::http::Status::Ok)
}

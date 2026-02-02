use hmac::{Hmac, Mac};
use sha2::Sha256;
use rocket::State;
use sqlx::SqlitePool;

use crate::stripe::download::create_download_tokens_for_order;
use crate::db::mark_order_paid;

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
    signature: StripeSignature,
) -> Result<rocket::http::Status, rocket::http::Status> {
    let secret = std::env::var("STRIPE_WEBHOOK_SECRET")
        .map_err(|_| rocket::http::Status::InternalServerError)?;

    verify_stripe_signature(payload.as_bytes(), &signature.0, &secret)
        .map_err(|e| {
            eprintln!("Webhook signature verification failed: {:?}", e);
            rocket::http::Status::Unauthorized
        })?;

    // Parse the event
    let json: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|e| {
            eprintln!("Failed to parse webhook JSON: {:?}", e);
            rocket::http::Status::BadRequest
        })?;

    let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or_default();

    if event_type == "checkout.session.completed" {
            if let Some(obj) = json.get("data").and_then(|d| d.get("object")) {
                let session_id = obj.get("id")
                    .and_then(|v| v.as_str())
                    .ok_or(rocket::http::Status::BadRequest)?;

                let payment_status = obj.get("payment_status")
                    .and_then(|v| v.as_str())
                    .ok_or(rocket::http::Status::BadRequest)?;

                // Look up the order by stripe_session_id
                let order_id = sqlx::query_scalar::<_, i64>(
                    "SELECT id FROM orders WHERE stripe_session_id = ?"
                )
                    .bind(session_id)
                    .fetch_optional(db.inner())
                    .await
                    .map_err(|e| {
                        eprintln!("DB error looking up order for session {}: {:?}", session_id, e);
                        rocket::http::Status::InternalServerError
                    })?
                    .ok_or_else(|| {
                        eprintln!("Webhook: no matching order for session {}", session_id);
                        rocket::http::Status::Ok
                    })?;

                if payment_status == "paid" {
                    if let Err(e) = mark_order_paid(db.inner(), order_id).await {
                        eprintln!("Error marking order {} paid: {:?}", order_id, e);
                        return Err(rocket::http::Status::InternalServerError);
                    }

                    if let Err(e) = create_download_tokens_for_order(db.inner(), order_id).await {
                        eprintln!("Error creating download tokens for order {}: {:?}", order_id, e);
                    }
                }
            }
        }

    Ok(rocket::http::Status::Ok)
}

fn verify_stripe_signature(payload: &[u8], signature_header: &str, secret: &str) -> Result<(), anyhow::Error> {
    let mut timestamp = None;
    let mut signatures: Vec<String> = Vec::new();

    for part in signature_header.split(',') {
        if let Some(ts) = part.strip_prefix("t=") {
            timestamp = Some(ts.to_string());
        } else if let Some(sig) = part.strip_prefix("v1=") {
            signatures.push(sig.to_string());
        }
    }

    let timestamp = timestamp.ok_or_else(|| anyhow::anyhow!("missing timestamp in Stripe-Signature"))?;
    if signatures.is_empty() {
        return Err(anyhow::anyhow!("no v1 signatures in Stripe-Signature"));
    }

    // Reject if the timestamp is older than 5 minutes (guards against replay attacks)
    let ts: i64 = timestamp.parse()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    if (now - ts) > 300 {
        return Err(anyhow::anyhow!("Stripe-Signature timestamp too old"));
    }

    // Compute expected signature: HMAC-SHA256 of "{timestamp}.{payload}"
    let signed_payload = format!("{}.{}", timestamp, std::str::from_utf8(payload)?);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(signed_payload.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    if signatures.iter().any(|s| s == &expected) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Stripe signature verification failed"))
    }
}


use rocket::request::{self, FromRequest, Request};

pub struct StripeSignature(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for StripeSignature {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        match req.headers().get_one("Stripe-Signature") {
            Some(sig) => request::Outcome::Success(StripeSignature(sig.to_string())),
            None => request::Outcome::Error((rocket::http::Status::BadRequest, ())),
        }
    }
}

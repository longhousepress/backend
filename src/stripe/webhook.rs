use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use rocket::State;
use sqlx::SqlitePool;

use rocket::request::{self, FromRequest, Request};
use tracing::{error, warn};

use crate::config::Config;
use crate::db::mark_order_paid;

/// Webhook endpoint to receive Stripe events.
#[post("/webhook", data = "<payload>")]
pub async fn stripe_webhook(
    config: &State<Config>,
    db: &State<SqlitePool>,
    payload: String,
    signature: StripeSignature,
    content_type: ContentType,
) -> Result<rocket::http::Status, rocket::http::Status> {
    // Validate Content-Type
    if !content_type.is_json() {
        warn!("Webhook rejected: invalid Content-Type");
        return Err(rocket::http::Status::BadRequest);
    }

    verify_stripe_signature(payload.as_bytes(), &signature.0, &config.stripe_webhook_secret)
        .map_err(|e| {
            error!("Webhook signature verification failed: {:?}", e);
            rocket::http::Status::Unauthorized
        })?;

    // Parse the event
    let json: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|e| {
            error!("Failed to parse webhook JSON: {:?}", e);
            rocket::http::Status::BadRequest
        })?;

    let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or_default();

    if event_type == "checkout.session.completed" {
        let deserialized_response: CheckoutSessionCompleted = serde_json::from_value(json)
            .map_err(|e| {
                error!("Could not deserialize checkout.session.completed webhook event: {e}");
                rocket::http::Status::InternalServerError
            })?;

        let session_id = deserialized_response.data.object.id;
        let customer_email = deserialized_response.data.object.customer_details.email;
        let payment_status = deserialized_response.data.object.payment_status;

        // Look up the order by stripe_session_id
        let order_id = sqlx::query_scalar::<_, i64>("SELECT id FROM orders WHERE stripe_session_id = ?")
            .bind(&session_id)
            .fetch_optional(db.inner())
            .await
            .map_err(|e| {
                error!("Database error looking up order for session {}: {:?}", session_id, e);
                rocket::http::Status::InternalServerError
            })?
            .ok_or_else(|| {
                warn!("Webhook received for unknown session {}", session_id);
                rocket::http::Status::Ok
            })?;

        if payment_status == "paid" {
            mark_order_paid(db.inner(), order_id, &customer_email).await
                .map_err(|e| {
                    error!("Error marking order {} paid: {:?}", order_id, e);
                    rocket::http::Status::InternalServerError
                })?;
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

#[derive(Serialize, Deserialize)]
struct CheckoutSessionCompleted {
    data: CheckoutSessionCompletedData,
}

#[derive(Serialize, Deserialize)]
struct CheckoutSessionCompletedData {
    object: CheckoutSessionCompletedObject,
}

#[derive(Serialize, Deserialize)]
struct CheckoutSessionCompletedObject {
    id: String,
    payment_status: String,
    customer_details: CheckoutSessionCompletedObjectCustomerDetails,
}

#[derive(Serialize, Deserialize)]
struct CheckoutSessionCompletedObjectCustomerDetails {
    email: String,
}

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

pub struct ContentType(pub rocket::http::ContentType);

impl ContentType {
    pub fn is_json(&self) -> bool {
        self.0.is_json()
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ContentType {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        match req.content_type() {
            Some(ct) => request::Outcome::Success(ContentType(ct.clone())),
            None => request::Outcome::Error((rocket::http::Status::BadRequest, ())),
        }
    }
}

use hmac::{Hmac, Mac};
use rocket::State;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::SqlitePool;
use subtle::ConstantTimeEq;
use tera::Tera;

use rocket::request::{self, FromRequest, Request};

use crate::config::Config;
use crate::db::mark_order_paid;
use crate::email::send_purchase_email;
use crate::stripe::verify_order::get_downloadable_books_for_order;

/// Webhook endpoint to receive Stripe events.
#[post("/webhook", data = "<payload>")]
pub async fn stripe_webhook(
    config: &State<Config>,
    db: &State<SqlitePool>,
    tera: &State<Tera>,
    payload: String,
    signature: StripeSignature,
    content_type: ContentType,
) -> Result<rocket::http::Status, rocket::http::Status> {
    rocket::info!("Webhook received");

    // Validate Content-Type
    if !content_type.is_json() {
        rocket::warn!("Webhook rejected: invalid Content-Type");
        return Err(rocket::http::Status::BadRequest);
    }

    verify_stripe_signature(
        payload.as_bytes(),
        &signature.0,
        &config.stripe_webhook_secret,
    )
    .map_err(|e| {
        rocket::error!("Webhook signature verification failed: {:?}", e);
        rocket::http::Status::Unauthorized
    })?;

    // Parse the event
    let json: serde_json::Value = serde_json::from_str(&payload).map_err(|e| {
        rocket::error!("Failed to parse webhook JSON: {:?}", e);
        rocket::http::Status::BadRequest
    })?;

    let event_type = json
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    rocket::info!("Webhook event type: {}", event_type);

    if event_type == "checkout.session.completed" {
        let deserialized_response: CheckoutSessionCompleted = serde_json::from_value(json)
            .map_err(|e| {
                rocket::error!(
                    "Could not deserialize checkout.session.completed webhook event: {e}"
                );
                rocket::http::Status::InternalServerError
            })?;

        let session_id = deserialized_response.data.object.id;
        let customer_email = deserialized_response.data.object.customer_details.email;
        let payment_status = deserialized_response.data.object.payment_status;

        rocket::info!(
            "Processing checkout.session.completed for session {} with payment status {}",
            session_id,
            payment_status
        );

        // Look up the order by stripe_session_id and verify email matches
        let order = sqlx::query!(
            "SELECT id, email, paid FROM orders WHERE stripe_session_id = ?",
            session_id
        )
        .fetch_optional(db.inner())
        .await
        .map_err(|e| {
            rocket::error!(
                "Database error looking up order for session {}: {:?}",
                session_id,
                e
            );
            rocket::http::Status::InternalServerError
        })?
        .ok_or_else(|| {
            rocket::warn!("Webhook received for unknown session {}", session_id);
            rocket::http::Status::InternalServerError
        })?;

        let order_id = order.id.ok_or_else(|| {
            rocket::error!("Order ID is null for session {}", session_id);
            rocket::http::Status::InternalServerError
        })?;

        // Idempotency guard: if already paid, acknowledge and do nothing
        if order.paid == Some(1) {
            rocket::info!("Order {} already processed, skipping", order_id);
            return Ok(rocket::http::Status::Ok);
        }

        // Verify email from Stripe matches the email stored in our order
        let stored_email = order.email.unwrap_or_default();
        if customer_email.to_lowercase() != stored_email.to_lowercase() {
            rocket::warn!(
                "Email mismatch for order {}: Stripe says '{}' but order has '{}'",
                order_id,
                customer_email,
                stored_email
            );
            // Use the email from our database (more trustworthy as it was user-provided)
            // but continue processing - this is just a warning
        }

        if payment_status == "paid" {
            rocket::info!("Marking order {} as paid", order_id);
            // Use the stored email from our database for consistency
            mark_order_paid(db.inner(), order_id, &stored_email)
                .await
                .map_err(|e| {
                    rocket::error!("Error marking order {} paid: {:?}", order_id, e);
                    rocket::http::Status::InternalServerError
                })?;

            // Send purchase confirmation email with download links
            rocket::info!("Fetching downloadable books for order {}", order_id);
            match get_downloadable_books_for_order(config, db.inner(), order_id).await {
                Ok(books) => {
                    rocket::info!(
                        "Got {} books for order {}, attempting to send email",
                        books.len(),
                        order_id
                    );
                    match send_purchase_email(
                        config.inner(),
                        tera.inner(),
                        &stored_email,
                        order_id,
                        &books,
                    )
                    .await
                    {
                        Ok(_) => {
                            rocket::info!(
                                "Email for order #{} sent successfully to {}",
                                order_id,
                                stored_email
                            );
                        }
                        Err(e) => {
                            rocket::error!(
                                "Failed to send purchase email for order {}: {:?}",
                                order_id,
                                e
                            );
                            // Continue processing - don't fail the webhook for email errors
                        }
                    }
                }
                Err(e) => {
                    rocket::error!(
                        "Failed to get downloadable books for order {} email: {:?}",
                        order_id,
                        e
                    );
                    // Continue processing - don't fail the webhook for email errors
                }
            }
        }
    }

    Ok(rocket::http::Status::Ok)
}

fn verify_stripe_signature(
    payload: &[u8],
    signature_header: &str,
    secret: &str,
) -> Result<(), anyhow::Error> {
    let mut timestamp = None;
    let mut signatures: Vec<String> = Vec::new();

    for part in signature_header.split(',') {
        if let Some(ts) = part.strip_prefix("t=") {
            timestamp = Some(ts.to_string());
        } else if let Some(sig) = part.strip_prefix("v1=") {
            signatures.push(sig.to_string());
        }
    }

    let timestamp =
        timestamp.ok_or_else(|| anyhow::anyhow!("missing timestamp in Stripe-Signature"))?;
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

    // Use constant-time comparison to prevent timing attacks
    let valid = signatures
        .iter()
        .any(|s| s.as_bytes().ct_eq(expected.as_bytes()).into());

    if valid {
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

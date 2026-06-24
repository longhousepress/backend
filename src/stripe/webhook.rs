use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::db::{find_order_by_session_id, get_downloadable_books_for_order, mark_order_paid};
use crate::email::send_purchase_email;
use crate::state::AppState;

/// Webhook endpoint to receive Stripe events.
pub async fn stripe_webhook(
    State(state): State<AppState>,
    signature: StripeSignature,
    headers: axum::http::HeaderMap,
    body: String,
) -> Result<StatusCode, StatusCode> {
    tracing::info!("Webhook received");

    // Validate Content-Type
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.contains("application/json") {
        tracing::warn!("Webhook rejected: invalid Content-Type");
        return Err(StatusCode::BAD_REQUEST);
    }

    verify_stripe_signature(
        body.as_bytes(),
        &signature.0,
        &state.config.stripe_webhook_secret,
    )
    .map_err(|e| {
        tracing::error!("Webhook signature verification failed: {:?}", e);
        StatusCode::UNAUTHORIZED
    })?;

    // Parse the event
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        tracing::error!("Failed to parse webhook JSON: {:?}", e);
        StatusCode::BAD_REQUEST
    })?;

    let event_type = json
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    tracing::info!("Webhook event type: {}", event_type);

    if event_type == "checkout.session.completed" {
        let deserialized_response: CheckoutSessionCompleted = serde_json::from_value(json)
            .map_err(|e| {
                tracing::error!(
                    "Could not deserialize checkout.session.completed webhook event: {e}"
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        let session_id = deserialized_response.data.object.id;
        let customer_email = deserialized_response.data.object.customer_details.email;
        let payment_status = deserialized_response.data.object.payment_status;

        tracing::info!(
            "Processing checkout.session.completed for session {} with payment status {}",
            session_id,
            payment_status
        );

        // Look up the order by stripe_session_id and verify email matches
        let order = match find_order_by_session_id(&state.db, &session_id).await {
            Err(e) => {
                tracing::error!(
                    "Database error looking up order for session {}: {:?}",
                    session_id,
                    e
                );
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
            Ok(None) => {
                // Unknown session — could be a test event or a pre-migration order.
                // Acknowledge with 200 so Stripe does not retry indefinitely.
                tracing::warn!(
                    "Webhook received for unknown session {}, acknowledging without processing",
                    session_id
                );
                return Ok(StatusCode::OK);
            }
            Ok(Some(o)) => o,
        };

        let order_id = order.id.ok_or_else(|| {
            tracing::error!("Order ID is null for session {}", session_id);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        // Idempotency guard: if already paid, acknowledge and do nothing
        if order.paid == Some(1) {
            tracing::info!("Order {} already processed, skipping", order_id);
            return Ok(StatusCode::OK);
        }

        // Verify email from Stripe matches the email stored in our order
        let stored_email = order.email.ok_or_else(|| {
            tracing::error!("Email is null for order {}", order_id);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        if let Some(ref stripe_email) = customer_email {
            if stripe_email.to_lowercase() != stored_email.to_lowercase() {
                tracing::warn!(
                    "Email mismatch for order {}: Stripe says '{}' but order has '{}'",
                    order_id,
                    stripe_email,
                    stored_email
                );
                // Use the email from our database (more trustworthy as it was user-provided)
                // but continue processing - this is just a warning
            }
        }

        if payment_status == "paid" {
            tracing::info!("Marking order {} as paid", order_id);
            // Use the stored email from our database for consistency
            mark_order_paid(&state.db, order_id, &stored_email)
                .await
                .map_err(|e| {
                    tracing::error!("Error marking order {} paid: {:?}", order_id, e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

            // Send purchase confirmation email with download links
            tracing::info!("Fetching downloadable books for order {}", order_id);
            match get_downloadable_books_for_order(&state.config, &state.db, order_id).await {
                Ok(books) => {
                    tracing::info!(
                        "Got {} books for order {}, attempting to send email",
                        books.len(),
                        order_id
                    );
                    match send_purchase_email(
                        &state.config,
                        &state.tera,
                        &stored_email,
                        order_id,
                        &books,
                    )
                    .await
                    {
                        Ok(_) => {
                            tracing::info!(
                                "Email for order #{} sent successfully to {}",
                                order_id,
                                stored_email
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to send purchase email for order {}: {:?}",
                                order_id,
                                e
                            );
                            // Continue processing - don't fail the webhook for email errors
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to get downloadable books for order {} email: {:?}",
                        order_id,
                        e
                    );
                    // Continue processing - don't fail the webhook for email errors
                }
            }
        }
    }

    Ok(StatusCode::OK)
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
    if (now - ts).abs() > 300 {
        return Err(anyhow::anyhow!("Stripe-Signature timestamp too old"));
    }

    // Compute expected signature: HMAC-SHA256 of "{timestamp}.{payload}"
    let signed_payload = format!("{}.{}", timestamp, std::str::from_utf8(payload)?);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(signed_payload.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    // Use constant-time comparison to prevent timing attacks.
    // Fold all comparisons with | before branching so short-circuit evaluation
    // can't leak whether an early signature matched.
    let valid: bool = signatures
        .iter()
        .fold(subtle::Choice::from(0u8), |acc, s| {
            acc | s.as_bytes().ct_eq(expected.as_bytes())
        })
        .into();

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
    email: Option<String>,
}

pub struct StripeSignature(pub String);

impl<S> FromRequestParts<S> for StripeSignature
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.headers.get("Stripe-Signature") {
            Some(sig) => Ok(StripeSignature(
                sig.to_str()
                    .map_err(|_| StatusCode::BAD_REQUEST)?
                    .to_string(),
            )),
            None => Err(StatusCode::BAD_REQUEST),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SECRET: &str = "whsec_test_secret_12345";

    fn make_signature_header(secret: &str, payload: &[u8], timestamp: i64) -> String {
        let signed_payload = format!("{}.{}", timestamp, std::str::from_utf8(payload).unwrap());
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed_payload.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        format!("t={},v1={}", timestamp, sig)
    }

    fn now_ts() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[test]
    fn valid_signature_accepts() {
        let payload = b"{\"type\":\"checkout.session.completed\"}";
        let header = make_signature_header(TEST_SECRET, payload, now_ts());
        assert!(verify_stripe_signature(payload, &header, TEST_SECRET).is_ok());
    }

    #[test]
    fn wrong_secret_rejects() {
        let payload = b"{\"type\":\"checkout.session.completed\"}";
        let header = make_signature_header(TEST_SECRET, payload, now_ts());
        assert!(verify_stripe_signature(payload, &header, "wrong_secret").is_err());
    }

    #[test]
    fn tampered_payload_rejects() {
        let payload = b"{\"type\":\"checkout.session.completed\"}";
        let header = make_signature_header(TEST_SECRET, payload, now_ts());
        let tampered = b"{\"type\":\"checkout.session.completed\",\"extra\":true}";
        assert!(verify_stripe_signature(tampered, &header, TEST_SECRET).is_err());
    }

    #[test]
    fn expired_timestamp_rejects() {
        let payload = b"{\"type\":\"checkout.session.completed\"}";
        let old_ts = now_ts() - 600; // 10 minutes ago
        let header = make_signature_header(TEST_SECRET, payload, old_ts);
        assert!(verify_stripe_signature(payload, &header, TEST_SECRET).is_err());
    }

    #[test]
    fn future_timestamp_rejects() {
        let payload = b"{\"type\":\"checkout.session.completed\"}";
        let future_ts = now_ts() + 600; // 10 minutes from now
        let header = make_signature_header(TEST_SECRET, payload, future_ts);
        assert!(verify_stripe_signature(payload, &header, TEST_SECRET).is_err());
    }

    #[test]
    fn timestamp_just_within_window_accepts() {
        let payload = b"{\"type\":\"checkout.session.completed\"}";
        let ts = now_ts() - 299; // 299 seconds ago, within 300s window
        let header = make_signature_header(TEST_SECRET, payload, ts);
        assert!(verify_stripe_signature(payload, &header, TEST_SECRET).is_ok());
    }

    #[test]
    fn missing_timestamp_rejects() {
        let payload = b"{}";
        let header = "v1=abcdef1234567890";
        assert!(verify_stripe_signature(payload, header, TEST_SECRET).is_err());
    }

    #[test]
    fn missing_v1_rejects() {
        let payload = b"{}";
        let header = format!("t={}", now_ts());
        assert!(verify_stripe_signature(payload, &header, TEST_SECRET).is_err());
    }

    #[test]
    fn empty_header_rejects() {
        let payload = b"{}";
        assert!(verify_stripe_signature(payload, "", TEST_SECRET).is_err());
    }

    #[test]
    fn multiple_v1_signatures_one_valid_accepts() {
        let payload = b"{\"type\":\"checkout.session.completed\"}";
        let ts = now_ts();
        let signed_payload = format!("{}.{}", ts, std::str::from_utf8(payload).unwrap());
        let mut mac = Hmac::<Sha256>::new_from_slice(TEST_SECRET.as_bytes()).unwrap();
        mac.update(signed_payload.as_bytes());
        let valid_sig = hex::encode(mac.finalize().into_bytes());
        let header = format!("t={},v1=deadbeef,v1={}", ts, valid_sig);
        assert!(verify_stripe_signature(payload, &header, TEST_SECRET).is_ok());
    }

    #[test]
    fn hex_case_mismatch_rejects() {
        let payload = b"{}";
        let ts = now_ts();
        let signed_payload = format!("{}.{}", ts, std::str::from_utf8(payload).unwrap());
        let mut mac = Hmac::<Sha256>::new_from_slice(TEST_SECRET.as_bytes()).unwrap();
        mac.update(signed_payload.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        // hex::encode is lowercase, so uppercase should not match
        let header = format!("t={},v1={}", ts, sig.to_uppercase());
        assert!(verify_stripe_signature(payload, &header, TEST_SECRET).is_err());
    }
}

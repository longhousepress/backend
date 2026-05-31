use crate::config::Config;
use crate::db::{find_order_by_session_id, get_downloadable_books_for_order};
use crate::models::Book;
use rocket::Request;
use rocket::State;
use rocket::http::Status;
use rocket::response::{Responder, Response};
use rocket::serde::json::Json;
use serde::Serialize;
use sqlx::SqlitePool;

/// Small responder type to send an HTTP status and optionally include the order id
/// in a custom header (used when returning 410 Gone).
pub enum ErrorResponse {
    Status(Status),
    WithOrder { status: Status, order_id: i64 },
}

impl<'r> Responder<'r, 'static> for ErrorResponse {
    fn respond_to(self, _req: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut rb = Response::build();
        match self {
            ErrorResponse::Status(s) => {
                rb.status(s);
            }
            ErrorResponse::WithOrder { status, order_id } => {
                rb.status(status);
                rb.raw_header("X-Order-Id", order_id.to_string());
            }
        }
        Ok(rb.finalize())
    }
}

// HTTP endpoint to verify an order's Stripe session and return downloadable metadata.
#[get("/order/verify?<session_id>")]
pub async fn verify_order_endpoint(
    config: &State<Config>,
    db: &State<SqlitePool>,
    session_id: String,
) -> std::result::Result<Json<SuccessReturn>, ErrorResponse> {
    // Look up the order by Stripe session id
    let row = find_order_by_session_id(db.inner(), &session_id)
        .await
        .map_err(|e| {
            rocket::error!(
                "Database error looking up order by session {}: {:?}",
                session_id,
                e
            );
            ErrorResponse::Status(Status::InternalServerError)
        })?
        .ok_or_else(|| {
            rocket::warn!("Order not found for session {}", session_id);
            ErrorResponse::Status(Status::NotFound)
        })?;

    // Extract order id early so we can include it in the Gone response header if needed
    let order_id = match row.id {
        Some(id) => id,
        None => return Err(ErrorResponse::Status(Status::InternalServerError)),
    };

    // Must be paid (webhook already validated this with Stripe)
    if row.paid != Some(1) {
        return Err(ErrorResponse::Status(Status::PaymentRequired));
    }

    // Check if the order was paid more than 90 minutes ago
    if let Some(paid_at_str) = row.paid_at {
        let paid_at = paid_at_str
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map_err(|e| {
                rocket::error!(
                    "Failed to parse paid_at timestamp for order {}: {:?}",
                    order_id,
                    e
                );
                ErrorResponse::Status(Status::InternalServerError)
            })?;

        let now = chrono::Utc::now();
        let elapsed = now.signed_duration_since(paid_at);

        if elapsed > chrono::Duration::minutes(90) {
            // Return 410 Gone with X-Order-Id header
            return Err(ErrorResponse::WithOrder {
                status: Status::Gone,
                order_id,
            });
        }
    }

    // Build downloadable books from the order
    let books = match get_downloadable_books_for_order(config, db.inner(), order_id).await {
        Ok(b) => b,
        Err(e) => {
            rocket::error!(
                "Error building downloadable metadata for order {}: {}",
                order_id,
                e
            );
            return Err(ErrorResponse::Status(Status::InternalServerError));
        }
    };

    let out = SuccessReturn {
        email: row.email.unwrap_or_default(),
        order_reference: order_id.to_string(),
        books,
    };

    Ok(Json(out))
}

#[derive(Serialize)]
pub struct SuccessReturn {
    pub email: String,
    pub order_reference: String,
    pub books: Vec<Book>,
}

use crate::db::{find_order_by_session_id, get_downloadable_books_for_order};
use crate::models::Book;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

/// Small responder type to send an HTTP status and optionally include the order id
/// in a custom header (used when returning 410 Gone).
pub enum ErrorResponse {
    Status(StatusCode),
    WithOrder { status: StatusCode, order_id: i64 },
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            ErrorResponse::Status(s) => s.into_response(),
            ErrorResponse::WithOrder { status, order_id } => {
                (status, [("X-Order-Id", order_id.to_string())]).into_response()
            }
        }
    }
}

#[derive(Deserialize)]
pub struct VerifyOrderParams {
    pub session_id: Option<String>,
}

// HTTP endpoint to verify an order's Stripe session and return downloadable metadata.
pub async fn verify_order_endpoint(
    State(state): State<AppState>,
    Query(params): Query<VerifyOrderParams>,
) -> Result<Json<SuccessReturn>, ErrorResponse> {
    let session_id = params.session_id.as_deref().ok_or_else(|| {
        ErrorResponse::Status(StatusCode::NOT_FOUND)
    })?;

    // Look up the order by Stripe session id
    let row = find_order_by_session_id(&state.db, session_id)
        .await
        .map_err(|e| {
            tracing::error!(
                "Database error looking up order by session {}: {:?}",
                session_id,
                e
            );
            ErrorResponse::Status(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            tracing::warn!("Order not found for session {}", session_id);
            ErrorResponse::Status(StatusCode::NOT_FOUND)
        })?;

    // Extract order id early so we can include it in the Gone response header if needed
    let order_id = match row.id {
        Some(id) => id,
        None => return Err(ErrorResponse::Status(StatusCode::INTERNAL_SERVER_ERROR)),
    };

    // Must be paid (webhook already validated this with Stripe)
    if row.paid != Some(1) {
        return Err(ErrorResponse::Status(StatusCode::PAYMENT_REQUIRED));
    }

    // Check if the order was paid more than 90 minutes ago
    if let Some(paid_at_str) = row.paid_at {
        let paid_at = paid_at_str
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map_err(|e| {
                tracing::error!(
                    "Failed to parse paid_at timestamp for order {}: {:?}",
                    order_id,
                    e
                );
                ErrorResponse::Status(StatusCode::INTERNAL_SERVER_ERROR)
            })?;

        let now = chrono::Utc::now();
        let elapsed = now.signed_duration_since(paid_at);

        if elapsed > chrono::Duration::minutes(90) {
            // Return 410 Gone with X-Order-Id header
            return Err(ErrorResponse::WithOrder {
                status: StatusCode::GONE,
                order_id,
            });
        }
    }

    // Build downloadable books from the order
    let books = match get_downloadable_books_for_order(&state.config, &state.db, order_id).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(
                "Error building downloadable metadata for order {}: {}",
                order_id,
                e
            );
            return Err(ErrorResponse::Status(StatusCode::INTERNAL_SERVER_ERROR));
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

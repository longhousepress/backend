use crate::config::Config;
use crate::models::{Book, Edition, File, FileFormat};
use anyhow::Result as AnyhowResult;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use rand::{RngCore, rng};
use rocket::Request;
use rocket::State;
use rocket::http::Status;
use rocket::response::{Responder, Response};
use rocket::serde::json::Json;
use serde::Serialize;
use sha2::Sha256;
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
#[get("/api/order/verify?<session_id>")]
pub async fn verify_order_endpoint(
    config: &State<Config>,
    db: &State<SqlitePool>,
    session_id: String,
) -> std::result::Result<Json<SuccessReturn>, ErrorResponse> {
    // Look up the order by Stripe session id
    let row = sqlx::query!(
        "SELECT id, paid, paid_at, email FROM orders WHERE stripe_session_id = ?",
        session_id
    )
    .fetch_one(db.inner())
    .await
    .map_err(|e| {
        rocket::error!(
            "Database error looking up order by session {}: {:?}",
            session_id, e
        );
        ErrorResponse::Status(Status::InternalServerError)
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

    // Check if the order was paid more than 15 minutes ago
    if let Some(paid_at_str) = row.paid_at {
        let paid_at = paid_at_str
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map_err(|e| {
                rocket::error!(
                    "Failed to parse paid_at timestamp for order {}: {:?}",
                    order_id, e
                );
                ErrorResponse::Status(Status::InternalServerError)
            })?;

        let now = chrono::Utc::now();
        let elapsed = now.signed_duration_since(paid_at);

        if elapsed > chrono::Duration::minutes(15) {
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
                order_id, e
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

/// Retrieve downloadable books for a given order.
/// Queries order_items directly to get all editions purchased, then builds
/// the downloadable metadata with minted tokens for each file.
pub async fn get_downloadable_books_for_order(
    config: &Config,
    db: &SqlitePool,
    order_id: i64,
) -> AnyhowResult<Vec<Book>> {
    // Query all editions for this order with book and author info
    let edition_rows = sqlx::query!(
        "SELECT
            e.id as \"id!: i64\",
            e.title as \"title!: String\",
            CAST(COALESCE(e.author_name, a.name) AS TEXT) as \"author_name!: String\",
            e.cover as \"cover!: String\",
            f.name as \"format!: String\",
            e.language as \"language: Option<String>\",
            b.slug as \"slug!: String\"
         FROM order_items oi
         INNER JOIN editions e ON oi.edition_id = e.id
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN authors a ON b.author_id = a.id
         INNER JOIN formats f ON e.format_id = f.id
         WHERE oi.order_id = ?",
        order_id
    )
    .fetch_all(db)
    .await?;

    if edition_rows.is_empty() {
        return Ok(Vec::new());
    }

    let mut books: Vec<Book> = Vec::with_capacity(edition_rows.len());

    for er in edition_rows {
        // Fetch files for this edition
        let file_rows = sqlx::query!(
            "SELECT ff.name as \"format_name!: String\", files.file_path as \"file_path!: String\"
             FROM files
             INNER JOIN file_formats ff ON files.file_format_id = ff.id
             WHERE files.edition_id = ?",
            er.id
        )
        .fetch_all(db)
        .await?;

        let mut files: Vec<File> = Vec::with_capacity(file_rows.len());
        for fr in file_rows {
            let fmt = match fr.format_name.as_str() {
                "epub" => FileFormat::Epub,
                "kepub" => FileFormat::Kepub,
                "azw3" => FileFormat::Azw3,
                "pdf" => FileFormat::Pdf,
                other => {
                    rocket::warn!(
                        "Unknown file format '{}' for edition {}, skipping",
                        other, er.id
                    );
                    continue; // skip unknown formats
                }
            };

            // Mint a download token on-demand for this filepath
            let token = mint(&fr.file_path, &config.token_key);
            let url = format!("/api/download/{}", token);
            files.push(File {
                format: fmt,
                path: url,
            });
        }

        // Build a minimal Edition
        let edition = Edition {
            id: er.id,
            title: er.title,
            author_name: er.author_name,
            author_bio: None,
            price: 0, // not relevant for downloads
            cover: er.cover,
            description: None,
            categories: Vec::new(),
            format: er.format,
            language: er.language.flatten(),
            page_count: None,
            translator: None,
            publication_date: None,
            isbn: None,
            edition_name: None,
            files: Some(files),
        };

        // Build a Book containing this edition
        let book = Book {
            id: er.id,
            title: edition.title.clone(),
            author: edition.author_name.clone(),
            book_slug: er.slug,
            editions: vec![edition],
        };

        books.push(book);
    }

    Ok(books)
}

// Mint a unique download token
pub fn mint(filepath: &str, token_key: &str) -> String {
    let mut payload = Vec::new();

    // 1) random nonce (16 bytes) - just for uniqueness
    let mut nonce = [0u8; 16];
    rng().fill_bytes(&mut nonce);
    payload.extend_from_slice(&nonce);

    // 2) filepath length (2 bytes) + filepath
    let path_bytes = filepath.as_bytes();
    payload.extend_from_slice(&(path_bytes.len() as u16).to_be_bytes());
    payload.extend_from_slice(path_bytes);

    // 3) sign with HMAC-SHA256
    let mut mac = Hmac::<Sha256>::new_from_slice(token_key.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(&payload);
    let sig = mac.finalize().into_bytes();
    payload.extend_from_slice(&sig);

    URL_SAFE_NO_PAD.encode(&payload)
}

#[derive(Serialize)]
pub struct SuccessReturn {
    pub email: String,
    pub order_reference: String,
    pub books: Vec<Book>,
}

use crate::config::Config;
use crate::models::{Book, Edition, File, FileFormat};
use crate::tokens::mint;
use anyhow::Result as AnyhowResult;
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
    let row = sqlx::query!(
        "SELECT id, paid, paid_at, email FROM orders WHERE stripe_session_id = ?",
        session_id
    )
    .fetch_one(db.inner())
    .await
    .map_err(|e| {
        rocket::error!(
            "Database error looking up order by session {}: {:?}",
            session_id,
            e
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

/// Retrieve downloadable books for a given order.
/// Queries order_items directly to get all editions purchased, then builds
/// the downloadable metadata with minted tokens for each file.
/// Respects quantity - if quantity > 1, returns multiple Book objects each with unique tokens.
pub async fn get_downloadable_books_for_order(
    config: &Config,
    db: &SqlitePool,
    order_id: i64,
) -> AnyhowResult<Vec<Book>> {
    // Query all order items with their quantities and edition info
    // Using GROUP_CONCAT to handle multiple authors per book
    let order_item_rows = sqlx::query!(
        "SELECT
            oi.quantity as \"quantity!: i64\",
            e.id as \"edition_id!: i64\",
            b.id as \"book_id!: i64\",
            bl.title as \"title!: String\",
            GROUP_CONCAT(pl.name, ', ') as \"author_names!: String\",
            e.cover_filepath as \"cover!: String\",
            f.name as \"format!: String\",
            e.language as \"language!: String\",
            b.original_language as \"original_language!: String\",
            b.slug as \"slug!: String\"
         FROM order_items oi
         INNER JOIN editions e ON oi.edition_id = e.id
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN book_localizations bl ON bl.book_id = b.id AND bl.language = e.language
         INNER JOIN formats f ON e.format_id = f.id
         LEFT JOIN book_contributors bc ON bc.book_id = b.id
         LEFT JOIN roles r ON bc.role_id = r.id AND r.name = 'Author'
         LEFT JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = e.language
         WHERE oi.order_id = ?
         GROUP BY oi.id, oi.quantity, e.id, b.id, bl.title, e.cover_filepath, f.name, e.language, b.slug, b.original_language
         ORDER BY b.id, e.id",
        order_id
    )
    .fetch_all(db)
    .await?;

    if order_item_rows.is_empty() {
        return Ok(Vec::new());
    }

    // Each order_item with quantity N gets expanded into N separate books
    // This ensures each "copy" gets unique download tokens
    let mut books: Vec<Book> = Vec::new();

    for oi_row in order_item_rows {
        // Repeat for the quantity purchased (e.g., if quantity = 2, create 2 book objects)
        for _ in 0..oi_row.quantity {
            // Fetch files for this edition (each iteration gets fresh tokens)
            let file_rows = sqlx::query!(
                "SELECT ff.name as \"format_name!: String\", files.file_path as \"file_path!: String\"
                 FROM files
                 INNER JOIN file_formats ff ON files.file_format_id = ff.id
                 WHERE files.edition_id = ? AND ff.name != 'sample'",
                oi_row.edition_id
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
                    "cover" => FileFormat::Cover,
                    other => {
                        rocket::warn!(
                            "Unknown file format '{}' for edition {}, skipping",
                            other,
                            oi_row.edition_id
                        );
                        continue; // skip unknown formats
                    }
                };

                // Mint a download token on-demand for this filepath
                // Each iteration creates unique tokens even for the same file
                let token = mint(&fr.file_path, &config.token_key);
                let url = format!("/api/download/{}", token);
                files.push(File {
                    format: fmt,
                    path: url,
                });
            }

            // Build a minimal Edition
            let edition = Edition {
                id: oi_row.edition_id,
                title: oi_row.title.clone(),
                author_name: oi_row.author_names.clone(),
                author_bio: None,
                prices: Vec::new(),
                cover: oi_row.cover.clone(),
                cover_name: None,
                cover_artist: None,
                description: None,
                categories: Vec::new(),
                format: oi_row.format.clone(),
                language: Some(oi_row.language.clone()),
                page_count: None,
                translator_name: None,
                illustrator: None,
                introduction_writer: None,
                contributors: Vec::new(),
                publication_date: None,
                isbn: None,
                edition_name: None,
                edition_notes: None,
                files: Some(files),
                samples: None,
            };

            // Create a separate Book object for each quantity
            // This ensures customers can gift or distribute copies with unique tokens
            let book = Book {
                id: oi_row.book_id,
                title: oi_row.title.clone(),
                subtitle: None,
                author: oi_row.author_names.clone(),
                book_slug: oi_row.slug.clone(),
                original_language: oi_row.original_language.clone(),
                original_publication_year: None,
                contributors: Vec::new(),
                editions: vec![edition],
            };

            books.push(book);
        }
    }

    Ok(books)
}

#[derive(Serialize)]
pub struct SuccessReturn {
    pub email: String,
    pub order_reference: String,
    pub books: Vec<Book>,
}

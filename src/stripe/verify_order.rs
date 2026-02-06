use crate::config::Config;
use crate::models::{Book, Edition, File, FileFormat};
use crate::tokens::mint;
use anyhow::Result as AnyhowResult;
use rocket::Request;
use rocket::State;
use rocket::http::Status;
use rocket::response::{Responder, Response};
use rocket::serde::json::Json;
use sqlx::SqlitePool;
use serde::Serialize;

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
/// Groups editions by book_id so each book appears once with all its purchased editions.
pub async fn get_downloadable_books_for_order(
    config: &Config,
    db: &SqlitePool,
    order_id: i64,
) -> AnyhowResult<Vec<Book>> {
    // Query all editions for this order with book and author info
    // Using GROUP_CONCAT to handle multiple authors per book
    let edition_rows = sqlx::query!(
        "SELECT
            e.id as \"edition_id!: i64\",
            b.id as \"book_id!: i64\",
            bl.title as \"title!: String\",
            GROUP_CONCAT(pl.name, ', ') as \"author_names!: String\",
            e.cover_filepath as \"cover!: String\",
            f.name as \"format!: String\",
            e.language as \"language!: String\",
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
         GROUP BY e.id, b.id, bl.title, e.cover_filepath, f.name, e.language, b.slug
         ORDER BY b.id, e.id",
        order_id
    )
    .fetch_all(db)
    .await?;

    if edition_rows.is_empty() {
        return Ok(Vec::new());
    }

    // Group editions by book_id
    use std::collections::HashMap;
    let mut books_map: HashMap<i64, Book> = HashMap::new();

    for er in edition_rows {
        // Fetch files for this edition
        let file_rows = sqlx::query!(
            "SELECT ff.name as \"format_name!: String\", files.file_path as \"file_path!: String\"
             FROM files
             INNER JOIN file_formats ff ON files.file_format_id = ff.id
             WHERE files.edition_id = ? AND ff.name != 'sample'",
            er.edition_id
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
                        other, er.edition_id
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
            id: er.edition_id,
            title: er.title.clone(),
            author_name: er.author_names.clone(),
            author_bio: None,
            prices: Vec::new(),
            cover: er.cover,
            description: None,
            categories: Vec::new(),
            format: er.format.clone(),
            language: Some(er.language.clone()),
            page_count: None,
            translator_name: None,
            illustrator: None,
            introduction_writer: None,
            contributors: Vec::new(),
            publication_date: None,
            isbn: None,
            edition_name: None,
            edition_notes: None,
            cover_name: None,
            cover_artist: None,
            files: Some(files),
            samples: None,
        };

        // Add edition to the appropriate book, or create a new book entry
        books_map
            .entry(er.book_id)
            .or_insert_with(|| Book {
                id: er.book_id,
                title: er.title.clone(),
                subtitle: None,
                author: er.author_names.clone(),
                book_slug: er.slug.clone(),
                original_language: String::from("eng"), // default, not queried in this context
                original_publication_year: None,
                contributors: Vec::new(),
                editions: Vec::new(),
            })
            .editions
            .push(edition);
    }

    // Convert HashMap to Vec and sort by book_id for consistent ordering
    let mut books: Vec<Book> = books_map.into_iter().map(|(_, book)| book).collect();
    books.sort_by_key(|b| b.id);

    Ok(books)
}

#[derive(Serialize)]
pub struct SuccessReturn {
    pub email: String,
    pub order_reference: String,
    pub books: Vec<Book>,
}

use std::time::{SystemTime, UNIX_EPOCH};
use rand::{rng, RngCore};
use anyhow::Result as AnyhowResult;
use hmac_sha256::HMAC;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sqlx::SqlitePool;
use crate::State;
use crate::models::{Book, Edition, File, FileFormat};
use rocket::serde::json::Json;
use serde::Serialize;
use rocket::http::Status;

/// Mint a short-lived, URL-safe token (internal format)
pub fn mint() -> String {
    const TOKEN_TTL_SECS: u64 = 15 * 60; // 15 minutes
    let expire_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + TOKEN_TTL_SECS;

    let mut buf = [0u8; 56];
    // 1) random nonce
    rng().fill_bytes(&mut buf[..16]);
    // 2) expiry (8 bytes)
    buf[16..24].copy_from_slice(&expire_ts.to_be_bytes());
    // 3) sign (HMAC-SHA256)
    let secret = std::env::var("TOKEN_KEY").expect("TOKEN_KEY not set");
    let sig = HMAC::mac(&buf[..24], secret.as_bytes());
    buf[24..].copy_from_slice(&sig[..]);
    // 4) encode URL-safe without padding
    URL_SAFE_NO_PAD.encode(&buf)
}

/// Verify token structure, signature and expiry
pub fn verify(tok: &str) -> Result<(), String> {
    let buf = URL_SAFE_NO_PAD.decode(tok)
        .map_err(|_| "bad base64".to_string())?;
    if buf.len() != 56 { return Err("buf.len() is not 56".to_string()); }
    let secret = std::env::var("TOKEN_KEY").map_err(|_| "missing TOKEN_KEY".to_string())?;
    let sig = HMAC::mac(&buf[..24], secret.as_bytes());
    if sig.as_slice() != &buf[24..] { return Err("signature mismatch".to_string()); }

    let expire_ts = u64::from_be_bytes(buf[16..24].try_into().unwrap());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH).unwrap()
        .as_secs();

    if expire_ts >= now {
        Ok(())
    } else {
        Err("token expired".to_string())
    }
}

/// Create download tokens for every file on every edition sold in an order.
/// Idempotent: if tokens already exist for the order, do nothing.
pub async fn create_download_tokens_for_order(pool: &SqlitePool, order_id: i64) -> AnyhowResult<()> {
    // If tokens already exist for this order, do nothing
    let existing_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM download_tokens WHERE order_id = ?"
    )
    .bind(order_id)
    .fetch_one(pool)
    .await?;

    if existing_count > 0 {
        return Ok(());
    }

    // Get the edition IDs on the order
    let items = sqlx::query!("SELECT edition_id FROM order_items WHERE order_id = ?", order_id)
        .fetch_all(pool)
        .await?;

    for r in items {
        let edition_id = r.edition_id;
        // Find files for the edition
        let file_rows = sqlx::query!("SELECT id FROM files WHERE edition_id = ?", edition_id)
            .fetch_all(pool)
            .await?;

        for f in file_rows {
            let file_id = f.id;
            let token = mint();
            // Insert token with 7-day expiry (SQLite strftime)
            sqlx::query!(
                "INSERT INTO download_tokens (order_id, file_id, token, expires_at) \
                 VALUES (?, ?, ?, (strftime('%Y-%m-%dT%H:%M:%SZ','now','+7 days')))",
                order_id,
                file_id,
                token
            )
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}

/// HTTP endpoint to verify an order's Stripe session and return downloadable metadata.
/// Returns JSON array of minimal `Book` objects describing editions/files available for download.
///
/// Note: this handler returns a Rocket `Result<T, Status>` (not `anyhow::Result`).
#[get("/api/order/verify?<session_id>")]
pub async fn verify_order_endpoint(db: &State<SqlitePool>, session_id: String) -> std::result::Result<Json<SuccessReturn>, Status> {
    // Look up the order by Stripe session id
    let row = sqlx::query!("SELECT id, paid FROM orders WHERE stripe_session_id = ?", session_id)
        .fetch_one(db.inner())
        .await
        .map_err(|e| { eprintln!("db error: {:?}", e); Status::InternalServerError })?;

    // Must be paid
    if row.paid != Some(1) {
        return Err(Status::PaymentRequired);
    }

    let id = match row.id {
        Some(id) => id,
        None => return Err(Status::InternalServerError),
    };

    // Ensure download tokens exist
    if let Err(e) = create_download_tokens_for_order(db.inner(), id).await {
        eprintln!("Could not create download tokens for order {}: {}", id, e);
        return Err(Status::InternalServerError);
    }

    // Fetch Stripe checkout session to obtain customer email and client_reference_id (order reference)
    let stripe_resp_text = {
        let client = reqwest::Client::new();
        let res = client
            .get(format!("https://api.stripe.com/v1/checkout/sessions/{}", session_id))
            .header("Authorization", format!("Bearer {}", std::env::var("STRIPE_API_KEY").unwrap_or_default()))
            .send()
            .await
            .map_err(|e| { eprintln!("stripe API error: {:?}", e); Status::InternalServerError })?;

        if !res.status().is_success() {
            eprintln!("stripe API returned {}", res.status());
            return Err(Status::InternalServerError);
        }

        res.text().await.map_err(|e| { eprintln!("stripe resp text error: {:?}", e); Status::InternalServerError })?
    };

    let stripe_json: serde_json::Value = serde_json::from_str(&stripe_resp_text)
        .map_err(|e| { eprintln!("stripe JSON parse error: {:?}", e); Status::InternalServerError })?;

    // Determine customer email and order reference (prefer client_reference_id)
    let customer_email = stripe_json
        .get("customer_details")
        .and_then(|cd| cd.get("email"))
        .and_then(|v| v.as_str())
        .or_else(|| stripe_json.get("customer_email").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let client_ref = stripe_json
        .get("client_reference_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Fetch distinct book slugs for the order
    let slug_rows = sqlx::query!(
        "SELECT DISTINCT b.slug as \"slug!: String\"
         FROM order_items oi
         INNER JOIN editions e ON oi.edition_id = e.id
         INNER JOIN books b ON e.book_id = b.id
         WHERE oi.order_id = ?",
         id
    )
    .fetch_all(db.inner())
    .await
    .map_err(|e| { eprintln!("db error: {:?}", e); Status::InternalServerError })?;

    let mut books: Vec<Book> = Vec::new();
    for s in slug_rows {
        let slug = s.slug;
        match get_downloadable_book(db.inner(), &slug, id).await {
            Ok(mut bvec) => books.append(&mut bvec),
            Err(e) => {
                eprintln!("error building downloadable metadata for {}: {}", &slug, e);
                return Err(Status::InternalServerError);
            }
        }
    }

    // order_reference: prefer client_reference_id from Stripe, fall back to internal order id
    let order_reference = match client_ref {
        Some(c) => c,
        None => id.to_string(),
    };

    let out = SuccessReturn {
        email: customer_email,
        order_reference,
        books,
    };

    Ok(Json(out))
}

/// Retrieve a minimal Book representation for the given book slug.
/// Each returned Book includes editions with only the fields required for downloads:
/// cover, title, format, language, and `files` listing available file formats + paths.
pub async fn get_downloadable_book(db: &SqlitePool, book_slug: &str, order_id: i64) -> AnyhowResult<Vec<Book>> {
    // Query editions for the given book slug, include author_name fallback and format
    let edition_rows = sqlx::query!(
        "SELECT
            e.id as \"id!: i64\",
            e.title as \"title!: String\",
            CAST(COALESCE(e.author_name, a.name) AS TEXT) as \"author_name!: String\",
            e.cover as \"cover!: String\",
            f.name as \"format!: String\",
            e.language as \"language: Option<String>\"
         FROM editions e
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN authors a ON b.author_id = a.id
         INNER JOIN formats f ON e.format_id = f.id
         WHERE b.slug = ?",
        book_slug
    )
    .fetch_all(db)
    .await?;

    if edition_rows.is_empty() {
        return Ok(Vec::new());
    }

    let mut books: Vec<Book> = Vec::with_capacity(edition_rows.len());

    for er in edition_rows {
        // Fetch files for this edition, include file id so we can look up the order-specific token
        let file_rows = sqlx::query!(
            "SELECT ff.name as \"format_name!: String\", files.file_path as \"file_path!: String\", files.id as \"file_id!: i64\"
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
                    eprintln!("unknown file format '{}' for edition {}", other, er.id);
                    continue; // skip unknown formats
                }
            };

            // Look up the download token for this order and file
            let token_row = sqlx::query!(
                "SELECT token FROM download_tokens WHERE order_id = ? AND file_id = ?",
                order_id,
                fr.file_id
            )
            .fetch_optional(db)
            .await?;

            if let Some(tr) = token_row {
                let url = format!("/api/download/{}", tr.token);
                files.push(File { format: fmt, path: url });
            } else {
                // If no token exists for this order & file, skip it (shouldn't happen since tokens are created earlier)
                eprintln!("no download token for order {} file {}", order_id, fr.file_id);
                continue;
            }
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

        // Build a Book containing this edition (caller groups by slug)
        let book = Book {
            id: er.id,
            title: edition.title.clone(),
            author: edition.author_name.clone(),
            book_slug: book_slug.to_string(),
            editions: vec![edition],
        };

        books.push(book);
    }

    Ok(books)
}

#[derive(Serialize)]
pub struct SuccessReturn {
	pub email: String,
	pub order_reference: String,
	pub books: Vec<Book>,
}

use std::time::{SystemTime, UNIX_EPOCH};
use rand::{rng, RngCore};
use anyhow::Result;
use hmac_sha256::HMAC;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sqlx::SqlitePool;
use crate::State;
use rocket::serde::json::Json;
use serde_json::Value;
use rocket::http::Status;

pub fn mint() -> String {
    const TOKEN_TTL_SECS: u64 = 15 * 60; // 15 min
    let expire_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + TOKEN_TTL_SECS;

    let mut buf = [0u8; 56];
    // 1. random nonce
    rng().fill_bytes(&mut buf[..16]);
    // 2. expiry
    buf[16..24].copy_from_slice(&expire_ts.to_be_bytes());
    // 3. sign
    let secret = std::env::var("TOKEN_KEY").expect("TOKEN_KEY not set");
    let sig = HMAC::mac(&buf[..24], secret.as_bytes());
    buf[24..].copy_from_slice(&sig[..]);
    // 4. url-safe base64
    URL_SAFE_NO_PAD.encode(&buf)
}

pub fn verify(tok: &str) -> Result<(), String> {
    let buf = URL_SAFE_NO_PAD.decode(tok)
        .map_err(|_| "bad base64".to_string())?;
    if buf.len() != 56 { return Err("buf.len() is not 56".to_string()); }
    let secret = std::env::var("TOKEN_KEY").unwrap();
    let sig = HMAC::mac(&buf[..24], secret.as_bytes());
    if sig.as_slice() != &buf[24..] { return Err("signature mismatch".to_string()); }

    let expire_ts = u64::from_be_bytes(buf[16..24].try_into().unwrap());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH).unwrap()
        .as_secs();

    if expire_ts >= now {
        return Ok(());
    }

    Err("token expired".to_string())
}

/// Create time-limited download tokens for every edition on an order. Idempotent.
pub async fn create_download_tokens_for_order(pool: &SqlitePool, order_id: i64) -> Result<()> {
    // If tokens already exist for this order, do nothing (idempotent)
    let existing_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM download_tokens WHERE order_id = ?"
    )
    .bind(order_id)
    .fetch_one(pool)
    .await?;

    if existing_count > 0 {
        return Ok(());
    }

    // Fetch editions for this order
    let items = sqlx::query!("SELECT edition_id FROM order_items WHERE order_id = ?", order_id)
        .fetch_all(pool)
        .await?;

    for r in items {
        let edition_id = r.edition_id;
        // For each file associated with the edition, mint a token and insert referencing file_id
        let file_rows = sqlx::query!("SELECT id FROM files WHERE edition_id = ?", edition_id)
            .fetch_all(pool)
            .await?;

        for f in file_rows {
            let file_id = f.id;
            // Mint a token (reusing existing mint() function)
            let token = mint();

            // Insert token with 7-day expiry (RFC3339 format using SQLite strftime)
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

/// A convenience HTTP endpoint you can mount to verify an order's Stripe session on demand.
#[get("/api/order/verify?<session_id>")]
pub async fn verify_order_endpoint(db: &State<SqlitePool>, session_id: String) -> Result<Json<Value>, Status> {

	let row = sqlx::query!(
		"SELECT id, paid FROM orders WHERE stripe_session_id = ?",
		session_id
    )
    .fetch_one(db.inner())
    .await
    .map_err(|e| { eprintln!("db error: {:?}", e); Status::InternalServerError})?;

	// Check if paid
    if row.paid != Some(1) {
        return Err(Status::PaymentRequired)
    }

    let id = match row.id {
    	Some(id) => id,
     	None => return Err(Status::InternalServerError)
    };

    match create_download_tokens_for_order(db.inner(), id).await {
		Ok(_) => (),
		Err(e) => {
			eprintln!("Could not create download tokens for order {}, error: {}", &id, e);
			return Err(Status::InternalServerError)
		}
    }

    let rows = sqlx::query!(
        "SELECT dt.token, dt.expires_at, e.title \
         FROM download_tokens dt \
         INNER JOIN files f ON dt.file_id = f.id \
         INNER JOIN editions e ON f.edition_id = e.id \
         WHERE dt.order_id = ?",
         id
    )
    .fetch_all(db.inner())
    .await
    .map_err(|e| { eprintln!("db error: {:?}", e); rocket::http::Status::InternalServerError })?;

    let mut list = Vec::with_capacity(rows.len());
    for r in rows {
        let token = r.token;
        let url = format!("/api/download/{}", token);
        list.push(serde_json::json!({
            "token": token,
            "url": url,
            "title": r.title,
            "expires_at": r.expires_at
        }));
    }

    return Ok(Json(Value::Array(list)))
}

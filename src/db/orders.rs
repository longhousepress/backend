use anyhow::Result;
use chrono::Utc;
use sqlx::sqlite::SqlitePool;

pub async fn get_edition_name(id: i64, db: &SqlitePool) -> Result<String> {
    let title_opt = sqlx::query_scalar::<_, String>(
        "SELECT bl.title
         FROM editions e
         INNER JOIN book_localizations bl ON bl.book_id = e.book_id AND bl.language = e.language
         WHERE e.id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    match title_opt {
        Some(title) => Ok(title),
        None => {
            rocket::error!("Edition id {} not found when fetching name", id);
            Err(anyhow::anyhow!("edition id {} not found", id))
        }
    }
}

pub async fn get_edition_price(id: i64, currency: &str, db: &SqlitePool) -> Result<u32> {
    let price_opt = sqlx::query_scalar::<_, i64>(
        "SELECT price FROM edition_prices WHERE edition_id = ? AND currency = ?",
    )
    .bind(id)
    .bind(currency)
    .fetch_optional(db)
    .await?;
    match price_opt {
        Some(price) => Ok(price as u32),
        None => {
            rocket::error!(
                "Edition id {} not found when fetching price for currency {}",
                id,
                currency
            );
            Err(anyhow::anyhow!(
                "edition id {} not found for currency {}",
                id,
                currency
            ))
        }
    }
}

pub async fn mark_order_paid(pool: &SqlitePool, order_id: i64, email: &str) -> Result<()> {
    let now = Utc::now();
    sqlx::query!(
        "UPDATE orders SET paid = 1, paid_at = ?, email = ? WHERE id = ?",
        now,
        email,
        order_id
    )
    .execute(pool)
    .await?;

    rocket::info!("Marked order {} as paid", order_id);
    Ok(())
}

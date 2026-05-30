use anyhow::Result;
use chrono::Utc;
use sqlx::sqlite::SqlitePool;

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

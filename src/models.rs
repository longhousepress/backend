use serde::{Serialize, Deserialize};
use sqlx::FromRow;
use anyhow::Result;
use sqlx::SqlitePool;

// For catalog listing - all editions with filter-relevant fields
#[derive(Serialize, Deserialize, FromRow)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub price: i64,
    pub cover: String,
    pub book_slug: String,
    pub format: String,
    pub language: Option<String>,
}

// For individual book detail - all fields
#[derive(Serialize, Deserialize)]
pub struct BookDetail {
    pub book_slug: String,
    pub year_published: Option<i64>,
    pub author: String,
    pub author_bio: Option<String>,
    pub categories: Vec<String>,
    pub editions: Vec<Edition>,
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Edition {
    pub id: i64,
    pub title: String,
    pub author_name: String,
    pub price: i64,
    pub cover: String,
    pub description: Option<String>,
    pub format: String,
    pub language: Option<String>,
    pub page_count: Option<i64>,
    pub translator: Option<String>,
    pub publication_date: Option<String>,
    pub isbn: Option<String>,
    pub edition_name: Option<String>,
}

// What the front end POSTs to us
#[derive(Serialize, Deserialize, FromRow)]
pub struct CheckoutRequest {
    pub items: Vec<CheckoutItem>,
}

// What we will return to the front end
#[derive(Serialize, Deserialize, FromRow)]
pub struct CheckoutSession {
    pub url: String,
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct CheckoutItem {
    pub edition_id: i64,
    pub quantity: u32,
}

#[allow(dead_code)]
impl CheckoutRequest {
    /// Persist this checkout request as an `orders` row and associated `order_items`.
    ///
    /// Behavior:
    /// - Looks up each edition's current `price` and sums the total.
    /// - Inserts a row into `orders` (with optional `client_reference` and `stripe_session_id`).
    /// - Inserts one `order_items` row per `CheckoutItem`, recording `price_at_purchase`.
    /// - All work is performed inside a sqlx transaction to ensure atomicity: either the order
    ///   and all its order_items are inserted, or nothing is committed.
    ///
    /// Parameters:
    /// - `pool`: sqlite pool to use for the transaction.
    /// - `client_reference`: optional client reference (e.g., Stripe metadata).
    /// - `stripe_session_id`: optional Stripe Checkout session id.
    /// - `currency`: optional currency string to store on the order.
    pub async fn persist(
        &self,
        pool: &SqlitePool,
        client_reference: Option<&str>,
        stripe_session_id: Option<&str>,
        currency: Option<&str>,
    ) -> Result<i64> {
        // Start a transaction so all operations are atomic
        let mut tx = pool.begin().await?;

        // Compute total and validate edition ids within the transaction
        let mut total_amount: i64 = 0;
        for item in &self.items {
            let edition_id: i64 = item.edition_id;

            // Read the current price for this edition using the transaction
            let row = sqlx::query!("SELECT price FROM editions WHERE id = ?", edition_id)
                .fetch_one(&mut *tx)
                .await?;
            let price: i64 = row.price;
            total_amount += price * (item.quantity as i64);
        }

        // Insert the order (paid is NULL for pending) inside the transaction
        let res = sqlx::query(
            "INSERT INTO orders (client_reference, stripe_session_id, paid, total_amount, currency) VALUES (?, ?, NULL, ?, ?)",
        )
        .bind(client_reference)
        .bind(stripe_session_id)
        .bind(total_amount)
        .bind(currency)
        .execute(&mut *tx)
        .await?;
 
        let order_id = res.last_insert_rowid();

        // Insert order_items (capture price_at_purchase inside the same transaction)
        for item in &self.items {
            let edition_id: i64 = item.edition_id;

            // capture the price at purchase time again to ensure consistency
            let row = sqlx::query!("SELECT price FROM editions WHERE id = ?", edition_id)
                .fetch_one(&mut *tx)
                .await?;
            let price_at_purchase: i64 = row.price;

            sqlx::query(
                "INSERT INTO order_items (order_id, edition_id, quantity, price_at_purchase) VALUES (?, ?, ?, ?)",
            )
            .bind(order_id)
            .bind(edition_id)
            .bind(item.quantity as i64)
            .bind(price_at_purchase)
            .execute(&mut *tx)
            .await?;
        }
 
        // Commit the transaction; if commit fails the transaction will be rolled back.
        tx.commit().await?;

        Ok(order_id)
    }
}

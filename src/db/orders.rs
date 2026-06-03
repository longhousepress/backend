use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use sqlx::sqlite::SqlitePool;

use crate::config::Config;
use crate::models::{Book, Edition, File, FileFormat};
use crate::tokens::mint;

pub struct OrderRecord {
    pub id: Option<i64>,
    pub email: Option<String>,
    pub paid: Option<i64>,
    pub paid_at: Option<String>,
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

    tracing::info!("Marked order {} as paid", order_id);
    Ok(())
}

pub async fn create_order(
    pool: &SqlitePool,
    email: &str,
    stripe_session_id: &str,
    currency: &str,
    items: &[(i64, u8)],
) -> Result<i64> {
    let mut tx = pool.begin().await?;

    let mut total_amount: i64 = 0;
    let mut prices: Vec<i64> = Vec::with_capacity(items.len());
    for (edition_id, quantity) in items {
        let row = sqlx::query!(
            "SELECT price FROM edition_prices WHERE edition_id = ? AND currency = ?",
            edition_id,
            currency
        )
        .fetch_one(&mut *tx)
        .await?;
        let price: i64 = row.price;
        prices.push(price);

        let line_total = price
            .checked_mul(*quantity as i64)
            .ok_or_else(|| anyhow::anyhow!("Price overflow for edition {}", edition_id))?;

        total_amount = total_amount
            .checked_add(line_total)
            .ok_or_else(|| anyhow::anyhow!("Total amount overflow"))?;
    }

    let res = sqlx::query(
        "INSERT INTO orders (stripe_session_id, email, paid, total_amount, currency) VALUES (?, ?, NULL, ?, ?)",
    )
    .bind(stripe_session_id)
    .bind(email)
    .bind(total_amount)
    .bind(currency)
    .execute(&mut *tx)
    .await?;

    let order_id = res.last_insert_rowid();

    for ((edition_id, quantity), price_at_purchase) in items.iter().zip(prices) {
        sqlx::query(
            "INSERT INTO order_items (order_id, edition_id, quantity, price_at_purchase, currency_at_purchase) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(order_id)
        .bind(edition_id)
        .bind(*quantity as i64)
        .bind(price_at_purchase)
        .bind(currency)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(order_id)
}

pub async fn find_order_by_session_id(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Option<OrderRecord>> {
    let row = sqlx::query!(
        "SELECT id, email, paid, paid_at FROM orders WHERE stripe_session_id = ?",
        session_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| OrderRecord {
        id: r.id,
        email: r.email,
        paid: r.paid,
        paid_at: r.paid_at,
    }))
}

pub async fn get_downloadable_books_for_order(
    config: &Config,
    db: &SqlitePool,
    order_id: i64,
) -> Result<Vec<Book>> {
    let order_item_rows = sqlx::query!(
        "SELECT
            oi.quantity as \"quantity!: i64\",
            e.id as \"edition_id!: i64\",
            b.id as \"book_id!: i64\",
            bl.title as \"title!: String\",
            bl.short_description as \"short_description!: String\",
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
         GROUP BY oi.id, oi.quantity, e.id, b.id, bl.title, bl.short_description, e.cover_filepath, f.name, e.language, b.slug, b.original_language
         ORDER BY b.id, e.id",
        order_id
    )
    .fetch_all(db)
    .await?;

    if order_item_rows.is_empty() {
        return Ok(Vec::new());
    }

    let file_rows = sqlx::query!(
        "SELECT files.edition_id as \"edition_id!: i64\", ff.name as \"format_name!: String\", files.file_path as \"file_path!: String\"
         FROM files
         INNER JOIN file_formats ff ON files.file_format_id = ff.id
         WHERE files.edition_id IN (
             SELECT DISTINCT oi.edition_id FROM order_items oi WHERE oi.order_id = ?
         ) AND ff.name != 'sample'",
        order_id
    )
    .fetch_all(db)
    .await?;

    use std::collections::HashMap;
    let mut files_by_edition: HashMap<i64, Vec<_>> = HashMap::new();
    for fr in &file_rows {
        files_by_edition.entry(fr.edition_id).or_default().push(fr);
    }

    let mut books: Vec<Book> = Vec::new();

    for oi_row in order_item_rows {
        let file_rows = files_by_edition.get(&oi_row.edition_id).map(|v| v.as_slice()).unwrap_or_default();

        for _ in 0..oi_row.quantity {
            let files: Vec<File> = file_rows
                .iter()
                .filter_map(|fr| {
                    let fmt = match fr.format_name.as_str() {
                        "epub" => FileFormat::Epub,
                        "kepub" => FileFormat::Kepub,
                        "azw3" => FileFormat::Azw3,
                        "pdf" => FileFormat::Pdf,
                        "cover" => FileFormat::Cover,
                        other => {
                            tracing::warn!(
                                "Unknown file format '{}' for edition {}, skipping",
                                other,
                                oi_row.edition_id
                            );
                            return None;
                        }
                    };

                    let token = mint(&fr.file_path, &config.token_key);
                    Some(File {
                        format: fmt,
                        path: format!("/api/download/{}", token),
                    })
                })
                .collect();

            let edition = Edition {
                id: oi_row.edition_id,
                title: oi_row.title.clone(),
                author_name: oi_row.author_names.clone(),
                author_bio: None,
                prices: Vec::new(),
                cover: oi_row.cover.clone(),
                cover_name: None,
                cover_artist: None,
                short_description: oi_row.short_description.clone(),
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
                original: None,
                files: Some(files),
                samples: None,
            };

            books.push(Book {
                id: oi_row.book_id,
                title: oi_row.title.clone(),
                subtitle: None,
                author: oi_row.author_names.clone(),
                book_slug: oi_row.slug.clone(),
                original_language: oi_row.original_language.clone(),
                original_publication_year: None,
                contributors: Vec::new(),
                editions: vec![edition],
            });
        }
    }

    Ok(books)
}

pub async fn check_files_exist(edition_id: i64, static_dir: &str, db: &SqlitePool) -> Result<bool> {
    let rows = sqlx::query!(
        "SELECT files.file_path as \"file_path!: String\"
         FROM files
         INNER JOIN file_formats ff ON files.file_format_id = ff.id
         WHERE files.edition_id = ? AND ff.name IN ('epub', 'kepub', 'azw3', 'pdf')",
        edition_id
    )
    .fetch_all(db)
    .await?;

    if rows.is_empty() {
        return Ok(false);
    }

    for row in rows {
        let full_path = Path::new(static_dir).join(&row.file_path);
        if !full_path.exists() {
            tracing::warn!(
                "Missing file for edition {}: {}",
                edition_id,
                full_path.display()
            );
            return Ok(false);
        }
    }
    Ok(true)
}

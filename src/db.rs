use anyhow::Result;
use chrono::Utc;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tracing::{error, info};

use crate::models::{Book, Edition};

const DB_SCHEMA: &str = include_str!("../schema.sql");
const DB_PATH: &str = "db.sqlite3";

pub async fn load_db() -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(DB_PATH)
        .create_if_missing(true)
        .foreign_keys(true);

    let db = SqlitePool::connect_with(opts).await?;

    sqlx::query(DB_SCHEMA).execute(&db).await?;

    Ok(db)
}

pub async fn load_books(db: &SqlitePool, lang: Option<&str>) -> Result<Vec<Book>> {
    // Get one edition per book, preferring the requested language.
    // Construct `Edition` and `Book` manually because listing returns one
    // representative edition per book; populate edition fields we have and
    // leave other fields with sensible defaults for a catalog listing.
    let rows = sqlx::query!(
        "SELECT
            e.id as id,
            e.title as title,
            CAST(COALESCE(e.author_name, a.name) AS TEXT) as author,
            e.price as price,
            e.cover as cover,
            b.slug as book_slug,
            b.id as book_id,
            f.name as format,
            e.language as language
         FROM editions e
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN authors a ON b.author_id = a.id
         INNER JOIN formats f ON e.format_id = f.id
         WHERE e.id IN (
             SELECT COALESCE(
                 -- First try: requested language
                 (SELECT e1.id FROM editions e1
                  WHERE e1.book_id = b.id AND e1.language = ?
                  LIMIT 1),
                 -- Second try: English
                 (SELECT e2.id FROM editions e2
                  WHERE e2.book_id = b.id AND e2.language = 'eng'
                  LIMIT 1),
                 -- Last resort: first edition found
                 (SELECT e3.id FROM editions e3
                  WHERE e3.book_id = b.id
                  LIMIT 1)
             )
             FROM books b
         )
         ORDER BY b.id",
        lang
    )
    .fetch_all(db)
    .await?;

    let mut books: Vec<Book> = Vec::new();

    for r in rows {
        // Fetch categories for this book
        let cat_rows = sqlx::query!(
            "SELECT c.name
             FROM categories c
             INNER JOIN book_categories bc ON c.id = bc.category_id
             WHERE bc.book_id = ?",
            r.book_id
        )
        .fetch_all(db)
        .await?;

        let categories: Vec<String> = cat_rows.into_iter().map(|c| c.name).collect();

        // Build a minimal Edition from the selected columns to include in Book.editions
        let edition = Edition {
            id: r.id,
            title: r.title,
            author_name: r.author.clone(),
            author_bio: None,
            price: r.price,
            cover: r.cover,
            description: None,
            categories,
            format: r.format,
            language: r.language,
            page_count: None,
            translator: None,
            publication_date: None,
            isbn: None,
            edition_name: None,
            files: None,
        };

        books.push(Book {
            id: r.id,
            title: edition.title.clone(),
            author: edition.author_name.clone(),
            book_slug: r.book_slug,
            editions: vec![edition],
        });
    }

    Ok(books)
}

pub async fn get_book_by_slug(db: &SqlitePool, book_slug: &str) -> Result<Option<Book>> {
    // First, get the book & author info
    let book_row = sqlx::query(
        "SELECT
            b.slug as book_slug,
            a.name as author,
            a.bio as author_bio
         FROM books b
         INNER JOIN authors a ON b.author_id = a.id
         WHERE b.slug = ?",
    )
    .bind(book_slug)
    .fetch_optional(db)
    .await?;

    let Some(row) = book_row else {
        return Ok(None);
    };

    // Extract basic book columns
    let book_slug: String = row.try_get("book_slug")?;
    let author_opt: Option<String> = row.try_get("author")?;
    let author: String = author_opt.unwrap_or_default();
    // author_bio will be attached to each edition below
    let author_bio_raw: Option<Option<String>> = row.try_get("author_bio")?;
    let author_bio: Option<String> = author_bio_raw.flatten();

    // Get all editions for this book (we'll map rows -> Edition and attach categories)
    // Annotate column aliases with explicit types so `sqlx::query!` can infer correct Rust types.
    let edition_rows = sqlx::query!(
        "SELECT
            e.id as \"id!: i64\",
            e.title as \"title!: String\",
            CAST(COALESCE(e.author_name, a.name) AS TEXT) as \"author_name!: String\",
            e.price as \"price!: i64\",
            e.cover as \"cover!: String\",
            e.description as \"description: Option<String>\",
            f.name as \"format!: String\",
            e.language as \"language: Option<String>\",
            e.page_count as \"page_count: Option<i64>\",
            e.translator as \"translator: Option<String>\",
            e.publication_date as \"publication_date: Option<String>\",
            e.isbn as \"isbn: Option<String>\",
            e.edition_name as \"edition_name: Option<String>\"
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
        return Ok(None);
    }

    // Fetch categories for this book (categories are stored per book)
    let cat_rows = sqlx::query!(
        "SELECT c.name
         FROM categories c
         INNER JOIN book_categories bc ON c.id = bc.category_id
         INNER JOIN books b ON bc.book_id = b.id
         WHERE b.slug = ?",
        book_slug
    )
    .fetch_all(db)
    .await?;

    let categories: Vec<String> = cat_rows.into_iter().map(|r| r.name).collect();

    // Map the edition rows into Edition structs and attach author_bio and categories.
    // Some sqlx query! aliases can produce nested Option<Option<T>> for nullable columns;
    // flatten those so the Edition fields get Option<T>.
    let editions: Vec<Edition> = edition_rows
        .into_iter()
        .map(|r| Edition {
            id: r.id,
            title: r.title,
            author_name: r.author_name,
            // attach the author's bio from the book-level query
            author_bio: author_bio.clone(),
            price: r.price,
            cover: r.cover,
            // Flatten nested options that can arise from the query macro
            description: r.description.flatten(),
            categories: categories.clone(),
            format: r.format,
            language: r.language.flatten(),
            page_count: r.page_count.flatten(),
            translator: r.translator.flatten(),
            publication_date: r.publication_date.flatten(),
            isbn: r.isbn.flatten(),
            edition_name: r.edition_name.flatten(),
            files: None,
        })
        .collect();

    // Use the first edition as representative for top-level Book fields
    let rep = &editions[0];

    Ok(Some(Book {
        id: rep.id,
        title: rep.title.clone(),
        author,
        book_slug,
        editions,
    }))
}

// Useful things when creating a Stripe session
pub async fn get_edition_name(id: i64, db: &SqlitePool) -> Result<String> {
    // Look up the edition title by numeric id.
    let title_opt = sqlx::query_scalar::<_, String>("SELECT title FROM editions WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    match title_opt {
        Some(title) => Ok(title),
        None => {
            error!("Edition id {} not found when fetching name", id);
            Err(anyhow::anyhow!("edition id {} not found", id))
        }
    }
}

pub async fn get_edition_price(id: i64, db: &SqlitePool) -> Result<u32> {
    // Look up the edition price by numeric id.
    let price_opt = sqlx::query_scalar::<_, i64>("SELECT price FROM editions WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    match price_opt {
        Some(price) => Ok(price as u32),
        None => {
            error!("Edition id {} not found when fetching price", id);
            Err(anyhow::anyhow!("edition id {} not found", id))
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

    info!("Marked order {} as paid for {}", order_id, email);
    Ok(())
}

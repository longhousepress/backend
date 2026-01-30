use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use sqlx::Row;
use anyhow::Result;

use crate::models::{Book, BookDetail, Edition};

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
    // Get one edition per book, preferring the requested language
    let books = sqlx::query_as!(
        Book,
        "SELECT
            e.id as \"id!\",
            e.title as \"title!\",
            CAST(COALESCE(e.author_name, a.name) AS TEXT) as \"author!: String\",
            e.price as \"price!\",
            e.cover as \"cover!\",
            b.slug as \"book_slug!\",
            f.name as \"format!\",
            e.language
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

    Ok(books)
}

pub async fn get_book_by_slug(db: &SqlitePool, book_slug: &str) -> Result<Option<BookDetail>> {
    // First, get the book info
    let book_row = sqlx::query(
        "SELECT 
            b.slug as book_slug,
            b.year_published,
            a.name as author,
            a.bio as author_bio
         FROM books b
         INNER JOIN authors a ON b.author_id = a.id
         WHERE b.slug = ?"
    )
    .bind(book_slug)
    .fetch_optional(db)
    .await?;

    let Some(row) = book_row else {
        return Ok(None);
    };

    // Extract columns with runtime checks to avoid sqlx compile-time alias issues
    let book_slug: String = row.try_get("book_slug")?;
    let year_published: Option<i64> = row.try_get("year_published")?;
    // Author is expected to be present in our model; coerce missing author to empty string
    let author_opt: Option<String> = row.try_get("author")?;
    let author: String = author_opt.unwrap_or_default();
    // Some SQLite/driver combinations can produce nested NULL mapping (Option<Option<String>>).
    // Read the raw value and then flatten to ensure we produce an Option<String>.
    let author_bio_raw: Option<Option<String>> = row.try_get("author_bio")?;
    let author_bio: Option<String> = author_bio_raw.flatten();

    // Get all editions for this book
    let editions = sqlx::query_as!(
        Edition,
        "SELECT 
            e.id as \"id!\",
            e.title as \"title!\",
            CAST(COALESCE(e.author_name, a.name) AS TEXT) as \"author_name!: String\",
            e.price as \"price!\",
            e.cover as \"cover!\",
            e.description,
            f.name as \"format!\",
            e.language,
            e.page_count,
            e.translator,
            e.publication_date,
            e.isbn,
            e.edition_name
         FROM editions e
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN authors a ON b.author_id = a.id
         INNER JOIN formats f ON e.format_id = f.id
         WHERE b.slug = ?",
        book_slug
    )
    .fetch_all(db)
    .await?;

    // Fetch categories for this book
    let categories = sqlx::query!(
        "SELECT c.name
         FROM categories c
         INNER JOIN book_categories bc ON c.id = bc.category_id
         INNER JOIN books b ON bc.book_id = b.id
         WHERE b.slug = ?",
        book_slug
    )
    .fetch_all(db)
    .await?;

    let categories: Vec<String> = categories.into_iter().map(|r| r.name).collect();

    Ok(Some(BookDetail {
        book_slug,
        year_published,
        author,
        author_bio,
        categories,
        editions,
    }))
}

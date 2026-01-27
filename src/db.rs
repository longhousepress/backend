use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use anyhow::Result;

use crate::models::{Book, BookDetail};

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

pub async fn load_books(db: &SqlitePool) -> Result<Vec<Book>> {
    let books = sqlx::query_as!(
        Book,
        "SELECT 
            e.id,
            e.title,
            COALESCE(e.author_name, a.name) as author,
            e.price,
            e.cover,
            a.slug || '/' || e.slug as slug
         FROM editions e
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN authors a ON b.author_id = a.id"
    )
    .fetch_all(db)
    .await?;

    Ok(books)
}

pub async fn get_book_by_slug(db: &SqlitePool, slug: &str) -> Result<Option<BookDetail>> {
    // Split slug into author_slug and edition_slug
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() != 2 {
        return Ok(None);
    }
    let (author_slug, edition_slug) = (parts[0], parts[1]);

    // Query the book details
    let row = sqlx::query!(
        "SELECT 
            e.id as \"id!\",
            e.title as \"title!\",
            CAST(COALESCE(e.author_name, a.name) AS TEXT) as \"author!: String\",
            a.slug as \"author_slug!\",
            a.bio as author_bio,
            e.price as \"price!\",
            e.cover as \"cover!\",
            CAST(a.slug || '/' || e.slug AS TEXT) as \"slug!: String\",
            e.description,
            f.name as \"format!\",
            e.language,
            e.page_count,
            e.translator,
            b.year_published,
            e.publication_date,
            e.isbn
         FROM editions e
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN authors a ON b.author_id = a.id
         INNER JOIN formats f ON e.format_id = f.id
         WHERE a.slug = ? AND e.slug = ?",
        author_slug,
        edition_slug
    )
    .fetch_optional(db)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    // Fetch categories for this book
    let categories = sqlx::query!(
        "SELECT c.name
         FROM categories c
         INNER JOIN book_categories bc ON c.id = bc.category_id
         INNER JOIN books b ON bc.book_id = b.id
         INNER JOIN editions e ON e.book_id = b.id
         INNER JOIN authors a ON b.author_id = a.id
         WHERE a.slug = ? AND e.slug = ?",
        author_slug,
        edition_slug
    )
    .fetch_all(db)
    .await?;

    let categories: Vec<String> = categories.into_iter().map(|r| r.name).collect();

    Ok(Some(BookDetail {
        id: row.id,
        title: row.title,
        author: row.author,
        author_slug: row.author_slug,
        author_bio: row.author_bio,
        price: row.price,
        cover: row.cover,
        slug: row.slug,
        description: row.description,
        format: row.format,
        language: row.language,
        page_count: row.page_count,
        translator: row.translator,
        year_published: row.year_published,
        publication_date: row.publication_date,
        isbn: row.isbn,
        categories,
    }))
}

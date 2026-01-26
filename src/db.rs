use sqlx::{sqlite::{SqliteConnectOptions, SqlitePool}};
use anyhow::Result;

use crate::models::Book;

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
    let books = sqlx::query_as!(Book, "select * from books;")
        .fetch_all(db).await?;

    Ok(books)
}

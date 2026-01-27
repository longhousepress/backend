use serde::{Serialize, Deserialize};
use sqlx::FromRow;

// For catalog listing - minimal fields
#[derive(Serialize, Deserialize, FromRow)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub price: i64,
    pub cover: String,
    pub slug: String,
}

// For individual book detail - all fields
#[derive(Serialize, Deserialize)]
pub struct BookDetail {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub author_slug: String,
    pub author_bio: Option<String>,
    pub price: i64,
    pub cover: String,
    pub slug: String,
    pub description: Option<String>,
    pub format: String,
    pub language: Option<String>,
    pub page_count: Option<i64>,
    pub translator: Option<String>,
    pub year_published: Option<i64>,
    pub publication_date: Option<String>,
    pub isbn: Option<String>,
    pub categories: Vec<String>,
}

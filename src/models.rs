use serde::{Serialize, Deserialize};
use sqlx::FromRow;

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
    pub author_name: Option<String>,
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

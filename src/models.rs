use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// For catalog listing - all editions with filter-relevant fields
#[derive(Serialize, Deserialize, FromRow)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub book_slug: String,
    pub editions: Vec<Edition>,
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Edition {
    pub id: i64,
    pub title: String,
    pub author_name: String,
    pub author_bio: Option<String>,
    pub price: i64,
    pub cover: String,
    pub description: Option<String>,
    pub categories: Vec<String>,
    pub format: String,
    pub language: Option<String>,
    pub page_count: Option<i64>,
    pub translator: Option<String>,
    pub publication_date: Option<String>,
    pub isbn: Option<String>,
    pub edition_name: Option<String>,
    pub files: Option<Vec<File>>,
    pub samples: Option<Vec<File>>,
}

#[derive(Serialize, Deserialize)]
pub struct File {
    pub format: FileFormat,
    pub path: String,
}

#[derive(Serialize, Deserialize)]
pub enum FileFormat {
    Epub,
    Kepub,
    Azw3,
    Pdf,
    Sample,
}

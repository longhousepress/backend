use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// Contributor to a book or edition
#[derive(Serialize, Deserialize, Clone)]
pub struct Contributor {
    pub name: String,
    pub role: String,
    pub bio: Option<String>,
    pub birth_year: Option<i64>,
    pub death_year: Option<i64>,
}

// Price in a specific currency
#[derive(Serialize, Deserialize, Clone)]
pub struct Price {
    pub currency: String,
    pub amount: i64,
}

// For catalog listing - all editions with filter-relevant fields
#[derive(Serialize, Deserialize, FromRow)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub author: String,
    pub book_slug: String,
    pub original_language: String,
    pub original_publication_year: Option<i64>,
    pub contributors: Vec<Contributor>,
    pub editions: Vec<Edition>,
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Edition {
    pub id: i64,
    pub title: String,
    pub author_name: String,
    pub author_bio: Option<String>,
    pub prices: Vec<Price>,
    pub cover: String,
    pub cover_name: Option<String>,
    pub cover_artist: Option<String>,
    pub description: Option<String>,
    pub categories: Vec<String>,
    pub format: String,
    pub language: Option<String>,
    pub page_count: Option<i64>,
    pub translator_name: Option<String>,
    pub illustrator: Option<String>,
    pub introduction_writer: Option<String>,
    pub contributors: Vec<Contributor>,
    pub publication_date: Option<String>,
    pub isbn: Option<String>,
    pub edition_name: Option<String>,
    pub edition_notes: Option<String>,
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
    Cover,
}

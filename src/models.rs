use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "eng")]
    Eng,
    #[serde(rename = "bul")]
    Bul,
    #[serde(rename = "kor")]
    Kor,
}

impl Language {
    pub fn as_str(&self) -> &str {
        match self {
            Language::Eng => "eng",
            Language::Bul => "bul",
            Language::Kor => "kor",
        }
    }

    pub fn as_url_segment(&self) -> &str {
        match self {
            Language::Eng => "en",
            Language::Bul => "bg",
            Language::Kor => "ko",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Currency {
    #[serde(rename = "USD")]
    Usd,
    #[serde(rename = "EUR")]
    Eur,
    #[serde(rename = "GBP")]
    Gbp,
    #[serde(rename = "KRW")]
    Krw,
}

impl TryFrom<String> for Currency {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "USD" => Ok(Currency::Usd),
            "EUR" => Ok(Currency::Eur),
            "GBP" => Ok(Currency::Gbp),
            "KRW" => Ok(Currency::Krw),
            _ => Err(anyhow::anyhow!("Unknown currency: {}", s)),
        }
    }
}

impl Currency {
    pub fn as_str(&self) -> &str {
        match self {
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
            Currency::Gbp => "GBP",
            Currency::Krw => "KRW",
        }
    }
}

// Contributor to a book or edition
#[derive(Serialize, Deserialize, Clone)]
pub struct Contributor {
    pub role_id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub role: String,
    pub bio: Option<String>,
    pub birth_year: Option<i64>,
    pub death_year: Option<i64>,
}

// Price in a specific currency
#[derive(Serialize, Deserialize, Clone)]
pub struct Price {
    pub currency: Currency,
    pub amount: i64,
}

// For catalog listing - all editions with filter-relevant fields
#[derive(Serialize, Deserialize, Clone)]
pub struct Book {
    pub id: i64,
    pub book_slug: String,
    pub original_language: String,
    pub original_publication_year: Option<i64>,
    pub contributors: Vec<Contributor>,
    pub editions: Vec<Edition>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Edition {
    pub id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub author_name: String,
    pub author_bio: Option<String>,
    pub prices: Vec<Price>,
    pub cover: String,
    pub cover_name: Option<String>,
    pub cover_artist: Option<String>,
    pub short_description: String,
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
    pub original: Option<bool>,
    pub files: Option<Vec<File>>,
    pub samples: Option<Vec<File>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct File {
    pub format: FileFormat,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum FileFormat {
    Epub,
    Kepub,
    Azw3,
    Pdf,
    Sample,
    Cover,
}

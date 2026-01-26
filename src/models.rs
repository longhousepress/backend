use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub price: i64,
    pub cover: String,
    pub slug: String,
}

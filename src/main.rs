use std::{path::PathBuf, str::FromStr};
use serde::{Serialize, Deserialize};
use rocket::serde::json::Json;

#[macro_use] extern crate rocket;

#[get("/api/books")]
fn index() -> Json<Book> {
    let book = Book {
        id: 32u32,
        title: "Thought-dreams".to_string(),
        author: "D.K.".to_string(),
        price: 5f32,
        cover: PathBuf::from_str("/Users/seok/media/covers/organized/cover057.jpg").unwrap(),
        slug: "thought-dreams-d-k".to_string(),
    };

    Json(book)
}

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![index])
}

#[derive(Serialize, Deserialize)]
struct Book {
    id: u32,
    title: String,
    author: String,
    price: f32,
    cover: PathBuf,
    slug: String,
}

use crate::db::get_book_by_slug;
use rocket::{State, http::Status, serde::json::Json};
use sqlx::SqlitePool;

use crate::models::Book;

#[get("/books/<slug>", rank = 2)]
pub async fn book_detail(db: &State<SqlitePool>, slug: String) -> Result<Json<Book>, Status> {
    // Validate slug length to prevent DoS attacks
    if slug.len() > 200 {
        rocket::warn!("Rejected oversized slug of length {}", slug.len());
        return Err(Status::BadRequest);
    }
    
    // Basic slug validation - allow only alphanumeric, hyphens, and underscores
    if !slug.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        rocket::warn!("Rejected invalid slug characters: {}", slug);
        return Err(Status::BadRequest);
    }
    
    match get_book_by_slug(db, &slug).await {
        Ok(Some(book)) => Ok(Json(book)),
        Ok(None) => Err(Status::NotFound),
        Err(e) => {
            rocket::error!("Failed to fetch book by slug '{}': {}", slug, e);
            Err(Status::InternalServerError)
        }
    }
}

use anyhow::Result;
use chrono::Utc;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};


use crate::models::{Book, Edition};

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

pub async fn load_books(db: &SqlitePool, lang: Option<&str>) -> Result<Vec<Book>> {
    // Get one edition per book, preferring the requested language.
    let lang = lang.unwrap_or("eng");
    
    let rows = sqlx::query!(
        "SELECT
            e.id as \"id!: i64\",
            bl.title as \"title!: String\",
            bl.subtitle as \"subtitle: Option<String>\",
            pl.name as \"author!: String\",
            ep.price as \"price: Option<i64>\",
            e.cover_filepath as \"cover!: String\",
            e.cover_name as \"cover_name: Option<String>\",
            b.slug as \"book_slug!: String\",
            b.id as \"book_id!: i64\",
            b.original_language as \"original_language!: String\",
            b.original_publication_year as \"original_publication_year: Option<i64>\",
            f.name as \"format!: String\",
            e.language as \"language!: String\",
            e.edition_notes as \"edition_notes: Option<String>\"
         FROM editions e
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN book_localizations bl ON bl.book_id = b.id AND bl.language = ?
         INNER JOIN formats f ON e.format_id = f.id
         INNER JOIN book_contributors bc ON bc.book_id = b.id
         INNER JOIN roles r ON bc.role_id = r.id AND r.name = 'Author'
         INNER JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = ?
         LEFT JOIN edition_prices ep ON ep.edition_id = e.id AND ep.currency = 'GBP'
         WHERE e.listed = 1 AND e.id IN (
             SELECT COALESCE(
                 -- First try: requested language
                 (SELECT e1.id FROM editions e1
                  WHERE e1.book_id = b.id AND e1.language = ? AND e1.listed = 1
                  LIMIT 1),
                 -- Second try: English
                 (SELECT e2.id FROM editions e2
                  WHERE e2.book_id = b.id AND e2.language = 'eng' AND e2.listed = 1
                  LIMIT 1),
                 -- Last resort: first edition found
                 (SELECT e3.id FROM editions e3
                  WHERE e3.book_id = b.id AND e3.listed = 1
                  LIMIT 1)
             )
             FROM books b
         )
         ORDER BY bc.ordinal ASC NULLS LAST, b.id",
        lang,
        lang,
        lang
    )
    .fetch_all(db)
    .await?;

    let mut books: Vec<Book> = Vec::new();

    for r in rows {
        // Fetch categories for this book
        let cat_rows = sqlx::query!(
            "SELECT c.name
             FROM categories c
             INNER JOIN book_categories bc ON c.id = bc.category_id
             WHERE bc.book_id = ?",
            r.book_id
        )
        .fetch_all(db)
        .await?;

        let categories: Vec<String> = cat_rows.into_iter().map(|c| c.name).collect();

        // Fetch all book-level contributors
        let book_contributor_rows = sqlx::query!(
            "SELECT pl.name, r.name as role, pl.bio, p.birth_year, p.death_year, bc.ordinal
             FROM book_contributors bc
             INNER JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = ?
             INNER JOIN roles r ON bc.role_id = r.id
             INNER JOIN persons p ON bc.person_id = p.id
             WHERE bc.book_id = ?
             ORDER BY bc.ordinal ASC NULLS LAST",
            lang,
            r.book_id
        )
        .fetch_all(db)
        .await?;

        let book_contributors: Vec<crate::models::Contributor> = book_contributor_rows
            .into_iter()
            .map(|c| crate::models::Contributor {
                name: c.name,
                role: c.role,
                bio: c.bio,
                birth_year: c.birth_year,
                death_year: c.death_year,
            })
            .collect();

        // Fetch all edition-level contributors
        let edition_contributor_rows = sqlx::query!(
            "SELECT pl.name, r.name as role, pl.bio, p.birth_year, p.death_year, ec.ordinal
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id
             INNER JOIN persons p ON ec.person_id = p.id
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST",
            lang,
            r.id
        )
        .fetch_all(db)
        .await?;

        let edition_contributors: Vec<crate::models::Contributor> = edition_contributor_rows
            .iter()
            .map(|c| crate::models::Contributor {
                name: c.name.clone(),
                role: c.role.clone(),
                bio: c.bio.clone(),
                birth_year: c.birth_year,
                death_year: c.death_year,
            })
            .collect();

        // Fetch all prices for this edition
        let price_rows = sqlx::query!(
            "SELECT currency, price
             FROM edition_prices
             WHERE edition_id = ?",
            r.id
        )
        .fetch_all(db)
        .await?;

        let prices: Vec<crate::models::Price> = price_rows
            .into_iter()
            .map(|p| crate::models::Price {
                currency: p.currency,
                amount: p.price,
            })
            .collect();

        // Get translator if exists for this edition
        let translator_name = sqlx::query_scalar::<_, String>(
            "SELECT pl.name
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id AND r.name = 'Translator'
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST
             LIMIT 1"
        )
        .bind(&r.language)
        .bind(r.id)
        .fetch_optional(db)
        .await?;

        // Get cover artist if exists for this edition
        let cover_artist = sqlx::query_scalar::<_, String>(
            "SELECT pl.name
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id AND r.name = 'Cover Artist'
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST
             LIMIT 1"
        )
        .bind(&r.language)
        .bind(r.id)
        .fetch_optional(db)
        .await?;

        // Get illustrator if exists for this edition
        let illustrator = sqlx::query_scalar::<_, String>(
            "SELECT pl.name
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id AND r.name = 'Illustrator'
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST
             LIMIT 1"
        )
        .bind(&r.language)
        .bind(r.id)
        .fetch_optional(db)
        .await?;

        // Get introduction writer if exists for this edition
        let introduction_writer = sqlx::query_scalar::<_, String>(
            "SELECT pl.name
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id AND r.name = 'Introduction Writer'
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST
             LIMIT 1"
        )
        .bind(&r.language)
        .bind(r.id)
        .fetch_optional(db)
        .await?;

        // Build a minimal Edition from the selected columns to include in Book.editions
        let edition = Edition {
            id: r.id,
            title: r.title.clone(),
            author_name: r.author.clone(),
            author_bio: None,
            price: r.price.unwrap_or(0),
            prices,
            cover: r.cover.clone(),
            cover_name: r.cover_name.flatten(),
            cover_artist,
            description: None,
            categories,
            format: r.format.clone(),
            language: Some(r.language.clone()),
            page_count: None,
            translator_name,
            illustrator,
            introduction_writer,
            contributors: edition_contributors,
            publication_date: None,
            isbn: None,
            edition_name: None,
            edition_notes: r.edition_notes.flatten(),
            files: None,
            samples: None,
        };

        books.push(Book {
            id: r.id,
            title: edition.title.clone(),
            subtitle: r.subtitle.flatten(),
            author: edition.author_name.clone(),
            book_slug: r.book_slug,
            original_language: r.original_language,
            original_publication_year: r.original_publication_year.flatten(),
            contributors: book_contributors,
            editions: vec![edition],
        });
    }

    Ok(books)
}

pub async fn get_book_by_slug(db: &SqlitePool, book_slug: &str) -> Result<Option<Book>> {
    // First, verify the book exists and get basic info
    let book_row = sqlx::query!(
        "SELECT id, slug, original_language, original_publication_year FROM books WHERE slug = ?",
        book_slug
    )
    .fetch_optional(db)
    .await?;

    let Some(book) = book_row else {
        return Ok(None);
    };
    
    let book_id = book.id;
    let book_original_language = book.original_language;
    let book_original_publication_year = book.original_publication_year;

    // Get all editions for this book with localized content
    let edition_rows = sqlx::query!(
        "SELECT
            e.id as \"id!: i64\",
            e.cover_filepath as \"cover!: String\",
            e.cover_name as \"cover_name: Option<String>\",
            e.language as \"language!: String\",
            e.page_count as \"page_count: Option<i64>\",
            e.publication_date as \"publication_date: Option<String>\",
            e.isbn as \"isbn: Option<String>\",
            e.edition_name as \"edition_name: Option<String>\",
            e.edition_notes as \"edition_notes: Option<String>\",
            bl.title as \"title!: String\",
            bl.subtitle as \"subtitle: Option<String>\",
            bl.description as \"description: Option<String>\",
            f.name as \"format!: String\",
            pl.name as \"author_name!: String\",
            pl.bio as \"author_bio: Option<String>\",
            ep.price as \"price: Option<i64>\"
         FROM editions e
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN formats f ON e.format_id = f.id
         INNER JOIN book_localizations bl ON bl.book_id = b.id AND bl.language = e.language
         INNER JOIN book_contributors bc ON bc.book_id = b.id
         INNER JOIN roles r ON bc.role_id = r.id AND r.name = 'Author'
         INNER JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = e.language
         LEFT JOIN edition_prices ep ON ep.edition_id = e.id AND ep.currency = 'GBP'
         WHERE b.id = ? AND e.listed = 1
         ORDER BY bc.ordinal ASC NULLS LAST",
        book_id
    )
    .fetch_all(db)
    .await?;

    if edition_rows.is_empty() {
        return Ok(None);
    }

    // Fetch categories for this book (categories are stored per book)
    let cat_rows = sqlx::query!(
        "SELECT c.name
         FROM categories c
         INNER JOIN book_categories bc ON c.id = bc.category_id
         WHERE bc.book_id = ?",
        book_id
    )
    .fetch_all(db)
    .await?;

    let categories: Vec<String> = cat_rows.into_iter().map(|r| r.name).collect();

    // Fetch all book-level contributors
    let book_contributor_rows = sqlx::query!(
        "SELECT pl.name, r.name as role, pl.bio, p.birth_year, p.death_year, bc.ordinal
         FROM book_contributors bc
         INNER JOIN person_localizations pl ON pl.person_id = bc.person_id
         INNER JOIN roles r ON bc.role_id = r.id
         INNER JOIN persons p ON bc.person_id = p.id
         WHERE bc.book_id = ?
         ORDER BY bc.ordinal ASC NULLS LAST",
        book_id
    )
    .fetch_all(db)
    .await?;

    let book_contributors: Vec<crate::models::Contributor> = book_contributor_rows
        .into_iter()
        .map(|c| crate::models::Contributor {
            name: c.name,
            role: c.role,
            bio: c.bio,
            birth_year: c.birth_year,
            death_year: c.death_year,
        })
        .collect();

    // Map the edition rows into Edition structs
    let mut editions: Vec<Edition> = Vec::new();
    let mut book_subtitle: Option<String> = None;
    
    for r in edition_rows {
        // Store subtitle from first edition (same for all editions of a book)
        if book_subtitle.is_none() {
            book_subtitle = r.subtitle.flatten();
        }

        // Fetch all edition-level contributors
        let edition_contributor_rows = sqlx::query!(
            "SELECT pl.name, r.name as role, pl.bio, p.birth_year, p.death_year, ec.ordinal
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id
             INNER JOIN roles r ON ec.role_id = r.id
             INNER JOIN persons p ON ec.person_id = p.id
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST",
            r.id
        )
        .fetch_all(db)
        .await?;

        let edition_contributors: Vec<crate::models::Contributor> = edition_contributor_rows
            .into_iter()
            .map(|c| crate::models::Contributor {
                name: c.name,
                role: c.role,
                bio: c.bio,
                birth_year: c.birth_year,
                death_year: c.death_year,
            })
            .collect();

        // Fetch all prices for this edition
        let price_rows = sqlx::query!(
            "SELECT currency, price
             FROM edition_prices
             WHERE edition_id = ?",
            r.id
        )
        .fetch_all(db)
        .await?;

        let prices: Vec<crate::models::Price> = price_rows
            .into_iter()
            .map(|p| crate::models::Price {
                currency: p.currency,
                amount: p.price,
            })
            .collect();

        // Get translator if exists for this edition
        let translator_name = sqlx::query_scalar::<_, String>(
            "SELECT pl.name
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id AND r.name = 'Translator'
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST
             LIMIT 1"
        )
        .bind(&r.language)
        .bind(r.id)
        .fetch_optional(db)
        .await?;

        // Get cover artist if exists for this edition
        let cover_artist = sqlx::query_scalar::<_, String>(
            "SELECT pl.name
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id AND r.name = 'Cover Artist'
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST
             LIMIT 1"
        )
        .bind(&r.language)
        .bind(r.id)
        .fetch_optional(db)
        .await?;

        // Get illustrator if exists for this edition
        let illustrator = sqlx::query_scalar::<_, String>(
            "SELECT pl.name
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id AND r.name = 'Illustrator'
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST
             LIMIT 1"
        )
        .bind(&r.language)
        .bind(r.id)
        .fetch_optional(db)
        .await?;

        // Get introduction writer if exists for this edition
        let introduction_writer = sqlx::query_scalar::<_, String>(
            "SELECT pl.name
             FROM edition_contributors ec
             INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
             INNER JOIN roles r ON ec.role_id = r.id AND r.name = 'Introduction Writer'
             WHERE ec.edition_id = ?
             ORDER BY ec.ordinal ASC NULLS LAST
             LIMIT 1"
        )
        .bind(&r.language)
        .bind(r.id)
        .fetch_optional(db)
        .await?;

        // Fetch sample files for this edition
        let sample_rows = sqlx::query!(
            "SELECT files.file_path as \"file_path!: String\"
             FROM files
             INNER JOIN file_formats ff ON files.file_format_id = ff.id
             WHERE files.edition_id = ? AND ff.name = 'sample'",
            r.id
        )
        .fetch_all(db)
        .await?;

        let samples = if sample_rows.is_empty() {
            None
        } else {
            Some(
                sample_rows
                    .into_iter()
                    .map(|sr| crate::models::File {
                        format: crate::models::FileFormat::Sample,
                        path: sr.file_path,
                    })
                    .collect()
            )
        };

        editions.push(Edition {
            id: r.id,
            title: r.title,
            author_name: r.author_name,
            author_bio: r.author_bio.flatten(),
            price: r.price.flatten().unwrap_or(0),
            prices,
            cover: r.cover,
            cover_name: r.cover_name.flatten(),
            cover_artist,
            description: r.description.flatten(),
            categories: categories.clone(),
            format: r.format,
            language: Some(r.language),
            page_count: r.page_count.flatten(),
            translator_name,
            illustrator,
            introduction_writer,
            contributors: edition_contributors,
            publication_date: r.publication_date.flatten(),
            isbn: r.isbn.flatten(),
            edition_name: r.edition_name.flatten(),
            edition_notes: r.edition_notes.flatten(),
            files: None,
            samples,
        });
    }

    // Use the first edition as representative for top-level Book fields
    let rep = &editions[0];

    Ok(Some(Book {
        id: rep.id,
        title: rep.title.clone(),
        subtitle: book_subtitle,
        author: rep.author_name.clone(),
        book_slug: book_slug.to_string(),
        original_language: book_original_language,
        original_publication_year: book_original_publication_year,
        contributors: book_contributors,
        editions,
    }))
}

// Useful things when creating a Stripe session
pub async fn get_edition_name(id: i64, db: &SqlitePool) -> Result<String> {
    // Look up the edition title by numeric id via book_localizations
    let title_opt = sqlx::query_scalar::<_, String>(
        "SELECT bl.title 
         FROM editions e
         INNER JOIN book_localizations bl ON bl.book_id = e.book_id AND bl.language = e.language
         WHERE e.id = ?"
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    match title_opt {
        Some(title) => Ok(title),
        None => {
            rocket::error!("Edition id {} not found when fetching name", id);
            Err(anyhow::anyhow!("edition id {} not found", id))
        }
    }
}

pub async fn get_edition_price(id: i64, db: &SqlitePool) -> Result<u32> {
    // Look up the edition price by numeric id from edition_prices (defaulting to GBP)
    let price_opt = sqlx::query_scalar::<_, i64>(
        "SELECT price FROM edition_prices WHERE edition_id = ? AND currency = 'GBP'"
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    match price_opt {
        Some(price) => Ok(price as u32),
        None => {
            rocket::error!("Edition id {} not found when fetching price", id);
            Err(anyhow::anyhow!("edition id {} not found", id))
        }
    }
}

pub async fn mark_order_paid(pool: &SqlitePool, order_id: i64, email: &str) -> Result<()> {
    let now = Utc::now();
    sqlx::query!(
        "UPDATE orders SET paid = 1, paid_at = ?, email = ? WHERE id = ?",
        now,
        email,
        order_id
    )
    .execute(pool)
    .await?;

    rocket::info!("Marked order {} as paid for {}", order_id, email);
    Ok(())
}

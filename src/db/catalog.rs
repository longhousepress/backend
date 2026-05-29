use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use sqlx::sqlite::SqlitePool;

use crate::models::{Book, Contributor, Edition, File, FileFormat, Price};

pub async fn load_books(db: &SqlitePool, static_dir: &str) -> Result<Vec<Book>> {
    let rows = sqlx::query!(
        "SELECT
            e.id as \"id!: i64\",
            bl.title as \"title!: String\",
            bl.subtitle as \"subtitle: Option<String>\",
            bl.short_description as \"short_description!: String\",
            bl.description as \"description: Option<String>\",
            GROUP_CONCAT(pl.name, ', ') as \"author!: String\",
            e.cover_filepath as \"cover!: String\",
            e.cover_name as \"cover_name: Option<String>\",
            b.slug as \"book_slug!: String\",
            b.id as \"book_id!: i64\",
            b.original_language as \"original_language!: String\",
            b.original_publication_year as \"original_publication_year: Option<i64>\",
            f.name as \"format!: String\",
            e.language as \"language!: String\",
            e.page_count as \"page_count: Option<i64>\",
            e.publication_date as \"publication_date: Option<String>\",
            e.isbn as \"isbn: Option<String>\",
            e.edition_name as \"edition_name: Option<String>\",
            e.edition_notes as \"edition_notes: Option<String>\",
            e.original as \"original: Option<bool>\",
            (SELECT pl2.bio FROM book_contributors bc2
             INNER JOIN person_localizations pl2 ON pl2.person_id = bc2.person_id AND pl2.language = e.language
             INNER JOIN roles r2 ON bc2.role_id = r2.id AND r2.name = 'Author'
             WHERE bc2.book_id = b.id
             ORDER BY bc2.ordinal ASC NULLS LAST
             LIMIT 1) as \"author_bio: Option<String>\"
         FROM editions e
         INNER JOIN books b ON e.book_id = b.id
         INNER JOIN book_localizations bl ON bl.book_id = b.id AND bl.language = e.language
         INNER JOIN formats f ON e.format_id = f.id
         LEFT JOIN book_contributors bc ON bc.book_id = b.id
         LEFT JOIN roles r ON bc.role_id = r.id AND r.name = 'Author'
         LEFT JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = e.language
         WHERE e.listed = 1
         GROUP BY e.id, bl.title, bl.subtitle, bl.short_description, bl.description, e.cover_filepath, e.cover_name, b.slug, b.id, b.original_language, b.original_publication_year, f.name, e.language, e.page_count, e.publication_date, e.isbn, e.edition_name, e.edition_notes, e.original
         ORDER BY b.id, e.id"
    )
    .fetch_all(db)
    .await?;

    let mut books_map: HashMap<i64, Book> = HashMap::new();

    for r in rows {
        let edition_contributors =
            fetch_edition_contributors(r.id, &r.language, db).await?;
        let prices = fetch_edition_prices(r.id, db).await?;
        let (translator_name, cover_artist, illustrator, introduction_writer) =
            fetch_contributor_roles(r.id, db).await?;

        if !check_files_exist(r.id, static_dir, db).await? {
            continue;
        }

        let samples = fetch_edition_samples(r.id, db).await?;

        let edition = Edition {
            id: r.id,
            title: r.title.clone(),
            author_name: r.author.clone(),
            author_bio: r.author_bio.flatten(),
            prices,
            cover: r.cover.clone(),
            cover_name: r.cover_name.flatten(),
            cover_artist,
            short_description: r.short_description.clone(),
            description: r.description.flatten(),
            categories: Vec::new(),
            format: r.format.clone(),
            language: Some(r.language.clone()),
            page_count: r.page_count.flatten(),
            translator_name,
            illustrator,
            introduction_writer,
            contributors: edition_contributors,
            publication_date: r.publication_date.flatten(),
            isbn: r.isbn.flatten(),
            edition_name: r.edition_name.flatten(),
            edition_notes: r.edition_notes.flatten(),
            original: r.original.flatten(),
            files: None,
            samples,
        };

        let book = books_map.entry(r.book_id).or_insert_with(|| Book {
            id: r.book_id,
            title: r.title.clone(),
            subtitle: r.subtitle.clone().flatten(),
            author: r.author.clone(),
            book_slug: r.book_slug.clone(),
            original_language: r.original_language.clone(),
            original_publication_year: r.original_publication_year.flatten(),
            contributors: Vec::new(),
            editions: Vec::new(),
        });

        book.editions.push(edition);
    }

    enrich_books(&mut books_map, db).await?;

    let mut books: Vec<Book> = books_map.into_iter().map(|(_, book)| book).collect();
    books.sort_by_key(|b| b.id);
    Ok(books)
}

async fn fetch_edition_contributors(
    edition_id: i64,
    language: &str,
    db: &SqlitePool,
) -> Result<Vec<Contributor>> {
    let rows = sqlx::query!(
        "SELECT pl.name, p.slug, r.name as role, pl.bio, p.birth_year, p.death_year, ec.ordinal
         FROM edition_contributors ec
         INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = ?
         INNER JOIN roles r ON ec.role_id = r.id
         INNER JOIN persons p ON ec.person_id = p.id
         WHERE ec.edition_id = ?
         ORDER BY ec.ordinal ASC NULLS LAST",
        language,
        edition_id
    )
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|c| Contributor {
            name: c.name,
            slug: c.slug,
            role: c.role,
            bio: c.bio,
            birth_year: c.birth_year,
            death_year: c.death_year,
        })
        .collect())
}

async fn fetch_edition_prices(edition_id: i64, db: &SqlitePool) -> Result<Vec<Price>> {
    let rows = sqlx::query!(
        "SELECT currency, price FROM edition_prices WHERE edition_id = ?",
        edition_id
    )
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|p| Price {
            currency: p.currency,
            amount: p.price,
        })
        .collect())
}

async fn fetch_contributor_roles(
    edition_id: i64,
    db: &SqlitePool,
) -> Result<(Option<String>, Option<String>, Option<String>, Option<String>)> {
    let row = sqlx::query!(
        "SELECT translator_name, cover_artist_name, illustrator_name, introduction_writer_name
         FROM edition_contributor_roles
         WHERE edition_id = ?",
        edition_id
    )
    .fetch_optional(db)
    .await?;

    Ok(if let Some(r) = row {
        (
            r.translator_name,
            r.cover_artist_name,
            r.illustrator_name,
            r.introduction_writer_name,
        )
    } else {
        (None, None, None, None)
    })
}

pub async fn check_files_exist(edition_id: i64, static_dir: &str, db: &SqlitePool) -> Result<bool> {
    let rows = sqlx::query!(
        "SELECT files.file_path as \"file_path!: String\"
         FROM files
         INNER JOIN file_formats ff ON files.file_format_id = ff.id
         WHERE files.edition_id = ? AND ff.id IN (1, 2, 3, 4)",
        edition_id
    )
    .fetch_all(db)
    .await?;

    if rows.is_empty() {
        return Ok(false);
    }

    for row in rows {
        let full_path = Path::new(static_dir).join(&row.file_path);
        if !full_path.exists() {
            eprintln!(
                "Missing file for edition {}: {}",
                edition_id,
                full_path.display()
            );
            return Ok(false);
        }
    }
    Ok(true)
}

async fn fetch_edition_samples(edition_id: i64, db: &SqlitePool) -> Result<Option<Vec<File>>> {
    let rows = sqlx::query!(
        "SELECT files.file_path as \"file_path!: String\"
         FROM files
         INNER JOIN file_formats ff ON files.file_format_id = ff.id
         WHERE files.edition_id = ? AND ff.name = 'sample'",
        edition_id
    )
    .fetch_all(db)
    .await?;

    if rows.is_empty() {
        return Ok(None);
    }

    Ok(Some(
        rows.into_iter()
            .map(|r| File {
                format: FileFormat::Sample,
                path: r.file_path,
            })
            .collect(),
    ))
}

async fn enrich_books(books_map: &mut HashMap<i64, Book>, db: &SqlitePool) -> Result<()> {
    for (book_id, book) in books_map.iter_mut() {
        let language = book.editions[0]
            .language
            .clone()
            .unwrap_or_else(|| "eng".to_string());

        let cat_rows = sqlx::query!(
            "SELECT c.name
             FROM categories c
             INNER JOIN book_categories bc ON c.id = bc.category_id
             WHERE bc.book_id = ?",
            book_id
        )
        .fetch_all(db)
        .await?;

        let categories: Vec<String> = cat_rows.into_iter().map(|c| c.name).collect();

        let contributor_rows = sqlx::query!(
            "SELECT pl.name, p.slug, r.name as role, pl.bio, p.birth_year, p.death_year, bc.ordinal
             FROM book_contributors bc
             INNER JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = ?
             INNER JOIN roles r ON bc.role_id = r.id
             INNER JOIN persons p ON bc.person_id = p.id
             WHERE bc.book_id = ?
             ORDER BY bc.ordinal ASC NULLS LAST",
            language,
            book_id
        )
        .fetch_all(db)
        .await?;

        book.contributors = contributor_rows
            .into_iter()
            .map(|c| Contributor {
                name: c.name,
                slug: c.slug,
                role: c.role,
                bio: c.bio,
                birth_year: c.birth_year,
                death_year: c.death_year,
            })
            .collect();

        for edition in &mut book.editions {
            edition.categories = categories.clone();
        }
    }
    Ok(())
}

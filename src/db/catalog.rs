use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use sqlx::sqlite::SqlitePool;

use crate::models::{Book, Contributor, Edition, File, FileFormat, Price};
struct FileEntry {
    file_path: String,
    is_main: bool,
    is_sample: bool,
}

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

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Fetch all supporting data in parallel — one bulk query per concern, no per-edition loops.
    let (
        edition_contributors_map,
        edition_prices_map,
        contributor_roles_map,
        files_by_edition,
        book_categories_map,
        book_contributors_map,
    ) = tokio::try_join!(
        fetch_all_edition_contributors(db),
        fetch_all_edition_prices(db),
        fetch_all_contributor_roles(db),
        fetch_all_files(db),
        fetch_all_book_categories(db),
        fetch_all_book_contributors(db),
    )?;

    let mut books_map: HashMap<i64, Book> = HashMap::new();

    for r in rows {
        // Validate that all required files exist on disk.
        let has_valid_files = files_by_edition
            .get(&r.id)
            .map(|fs| {
                let main_files: Vec<_> = fs.iter().filter(|f| f.is_main).collect();
                if main_files.is_empty() {
                    return false;
                }
                main_files.iter().all(|f| {
                    let full_path = Path::new(static_dir).join(&f.file_path);
                    if !full_path.exists() {
                        rocket::warn!("Missing file for edition {}: {}", r.id, full_path.display());
                        false
                    } else {
                        true
                    }
                })
            })
            .unwrap_or(false);

        if !has_valid_files {
            continue;
        }

        let samples: Vec<File> = files_by_edition
            .get(&r.id)
            .map(|fs| {
                fs.iter()
                    .filter(|f| f.is_sample)
                    .map(|f| File {
                        format: FileFormat::Sample,
                        path: f.file_path.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let samples = if samples.is_empty() { None } else { Some(samples) };

        let prices = edition_prices_map.get(&r.id).cloned().unwrap_or_default();
        let contributors = edition_contributors_map.get(&r.id).cloned().unwrap_or_default();
        let (translator_name, cover_artist, illustrator, introduction_writer) =
            contributor_roles_map.get(&r.id).cloned().unwrap_or((None, None, None, None));
        let categories = book_categories_map.get(&r.book_id).cloned().unwrap_or_default();

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
            categories,
            format: r.format.clone(),
            language: Some(r.language.clone()),
            page_count: r.page_count.flatten(),
            translator_name,
            illustrator,
            introduction_writer,
            contributors,
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
            // Use the first edition's language for book-level contributor localisation,
            // matching the previous behaviour.
            contributors: book_contributors_map
                .get(&(r.book_id, r.language.clone()))
                .cloned()
                .unwrap_or_default(),
            editions: Vec::new(),
        });

        book.editions.push(edition);
    }

    let mut books: Vec<Book> = books_map.into_values().collect();
    books.sort_by_key(|b| b.id);
    Ok(books)
}

async fn fetch_all_edition_contributors(db: &SqlitePool) -> Result<HashMap<i64, Vec<Contributor>>> {
    let rows = sqlx::query!(
        "SELECT ec.edition_id as \"edition_id!: i64\",
                pl.name, p.slug, r.name as role, pl.bio, p.birth_year, p.death_year
         FROM edition_contributors ec
         INNER JOIN editions e ON ec.edition_id = e.id AND e.listed = 1
         INNER JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = e.language
         INNER JOIN roles r ON ec.role_id = r.id
         INNER JOIN persons p ON ec.person_id = p.id
         ORDER BY ec.edition_id, ec.ordinal ASC NULLS LAST"
    )
    .fetch_all(db)
    .await?;

    let mut map: HashMap<i64, Vec<Contributor>> = HashMap::new();
    for r in rows {
        map.entry(r.edition_id).or_default().push(Contributor {
            name: r.name,
            slug: r.slug,
            role: r.role,
            bio: r.bio,
            birth_year: r.birth_year,
            death_year: r.death_year,
        });
    }
    Ok(map)
}

async fn fetch_all_edition_prices(db: &SqlitePool) -> Result<HashMap<i64, Vec<Price>>> {
    let rows = sqlx::query!(
        "SELECT ep.edition_id as \"edition_id!: i64\", ep.currency, ep.price
         FROM edition_prices ep
         INNER JOIN editions e ON ep.edition_id = e.id AND e.listed = 1"
    )
    .fetch_all(db)
    .await?;

    let mut map: HashMap<i64, Vec<Price>> = HashMap::new();
    for r in rows {
        map.entry(r.edition_id).or_default().push(Price {
            currency: r.currency,
            amount: r.price,
        });
    }
    Ok(map)
}

async fn fetch_all_contributor_roles(
    db: &SqlitePool,
) -> Result<HashMap<i64, (Option<String>, Option<String>, Option<String>, Option<String>)>> {
    let rows = sqlx::query!(
        "SELECT ecr.edition_id as \"edition_id!: i64\",
                ecr.translator_name, ecr.cover_artist_name,
                ecr.illustrator_name, ecr.introduction_writer_name
         FROM edition_contributor_roles ecr
         INNER JOIN editions e ON ecr.edition_id = e.id AND e.listed = 1"
    )
    .fetch_all(db)
    .await?;

    let mut map = HashMap::new();
    for r in rows {
        map.insert(
            r.edition_id,
            (
                r.translator_name,
                r.cover_artist_name,
                r.illustrator_name,
                r.introduction_writer_name,
            ),
        );
    }
    Ok(map)
}

async fn fetch_all_files(db: &SqlitePool) -> Result<HashMap<i64, Vec<FileEntry>>> {
    let rows = sqlx::query!(
        "SELECT f.edition_id as \"edition_id!: i64\",
                f.file_path as \"file_path!: String\",
                ff.name as \"format_name!: String\"
         FROM files f
         INNER JOIN file_formats ff ON f.file_format_id = ff.id
         INNER JOIN editions e ON f.edition_id = e.id AND e.listed = 1
         WHERE ff.name IN ('epub', 'kepub', 'azw3', 'pdf', 'sample')"
    )
    .fetch_all(db)
    .await?;

    let mut map: HashMap<i64, Vec<FileEntry>> = HashMap::new();
    for r in rows {
        map.entry(r.edition_id).or_default().push(FileEntry {
            file_path: r.file_path,
            is_main: matches!(r.format_name.as_str(), "epub" | "kepub" | "azw3" | "pdf"),
            is_sample: r.format_name == "sample",
        });
    }
    Ok(map)
}

async fn fetch_all_book_categories(db: &SqlitePool) -> Result<HashMap<i64, Vec<String>>> {
    let rows = sqlx::query!(
        "SELECT DISTINCT bc.book_id as \"book_id!: i64\", c.name as \"name!: String\"
         FROM book_categories bc
         INNER JOIN categories c ON c.id = bc.category_id
         INNER JOIN editions e ON e.book_id = bc.book_id AND e.listed = 1"
    )
    .fetch_all(db)
    .await?;

    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    for r in rows {
        map.entry(r.book_id).or_default().push(r.name);
    }
    Ok(map)
}

// Keyed by (book_id, language) so each book uses the localisation of its first edition,
// matching the previous behaviour in enrich_books.
async fn fetch_all_book_contributors(
    db: &SqlitePool,
) -> Result<HashMap<(i64, String), Vec<Contributor>>> {
    let rows = sqlx::query!(
        "SELECT DISTINCT bc.book_id as \"book_id!: i64\",
                el.language as \"language!: String\",
                pl.name, p.slug, r.name as role, pl.bio, p.birth_year, p.death_year, bc.ordinal
         FROM book_contributors bc
         INNER JOIN (SELECT DISTINCT book_id, language FROM editions WHERE listed = 1) el
             ON el.book_id = bc.book_id
         INNER JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = el.language
         INNER JOIN roles r ON bc.role_id = r.id
         INNER JOIN persons p ON bc.person_id = p.id
         ORDER BY bc.book_id, bc.ordinal ASC NULLS LAST"
    )
    .fetch_all(db)
    .await?;

    let mut map: HashMap<(i64, String), Vec<Contributor>> = HashMap::new();
    for r in rows {
        map.entry((r.book_id, r.language)).or_default().push(Contributor {
            name: r.name,
            slug: r.slug,
            role: r.role,
            bio: r.bio,
            birth_year: r.birth_year,
            death_year: r.death_year,
        });
    }
    Ok(map)
}

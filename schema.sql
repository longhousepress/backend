-- Authors are independent entities (canonical form)
CREATE TABLE IF NOT EXISTS authors (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,            -- Canonical name (e.g., "Molière")
    bio TEXT,
    slug TEXT NOT NULL UNIQUE
);

-- The abstract "work" - the book as a concept (minimal identity)
CREATE TABLE IF NOT EXISTS books (
    id INTEGER PRIMARY KEY,
    author_id INTEGER NOT NULL,
    year_published INTEGER,        -- Original publication year
    FOREIGN KEY (author_id) REFERENCES authors(id) ON DELETE RESTRICT
);

-- Format lookup table (seeded values)
CREATE TABLE IF NOT EXISTS formats (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

-- Seed the formats
INSERT OR IGNORE INTO formats (name) VALUES 
    ('Paperback'),
    ('Hardcover'),
    ('eBook');

-- Your specific edition/publication of that work
CREATE TABLE IF NOT EXISTS editions (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL,
    format_id INTEGER NOT NULL,
    title TEXT NOT NULL,           -- Localized title for this edition
    author_name TEXT,              -- Localized author name (e.g., "몰리에르"), NULL falls back to authors.name
    isbn TEXT UNIQUE,
    price INTEGER NOT NULL,
    cover TEXT NOT NULL,
    slug TEXT NOT NULL,
    edition_name TEXT,             -- "Revised", "Anniversary", "Deluxe"
    translator TEXT,               -- Nullable for translations
    language TEXT,                 -- ISO 639-3 language + ISO 15924 script tags (e.g., 'eng', 'fra', 'lzh-Hant')
    publication_date TEXT,         -- ISO date for this edition
    page_count INTEGER,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (format_id) REFERENCES formats(id) ON DELETE RESTRICT
);

-- Categories/genres (many-to-many with books)
CREATE TABLE IF NOT EXISTS categories (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS book_categories (
    book_id INTEGER NOT NULL,
    category_id INTEGER NOT NULL,
    PRIMARY KEY (book_id, category_id),
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE CASCADE
);
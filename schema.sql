-- Persons are independent entities
CREATE TABLE IF NOT EXISTS persons (
    id INTEGER PRIMARY KEY,
    slug TEXT UNIQUE,
    birth_year INTEGER,
    death_year INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT
) STRICT;

-- Localized information about persons
CREATE TABLE IF NOT EXISTS person_localizations (
    id INTEGER PRIMARY KEY,
    person_id INTEGER NOT NULL,
    language TEXT NOT NULL,
    name TEXT NOT NULL,
    bio TEXT,
    UNIQUE (person_id, language),
    FOREIGN KEY (person_id) REFERENCES persons(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_person_localizations_person_id ON person_localizations(person_id);
CREATE INDEX IF NOT EXISTS idx_person_localizations_language ON person_localizations(language);

-- The abstract "work" - the book as a concept
CREATE TABLE IF NOT EXISTS books (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    original_language TEXT NOT NULL,
    original_publication_year INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT
) STRICT;

-- Localized information about books
CREATE TABLE IF NOT EXISTS book_localizations (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL,
    language TEXT NOT NULL,
    title TEXT NOT NULL,
    subtitle TEXT,
    description TEXT,
    UNIQUE (book_id, language),
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_book_localizations_book_id ON book_localizations(book_id);
CREATE INDEX IF NOT EXISTS idx_book_localizations_language ON book_localizations(language);

-- Roles lookup table (Author, Translator, Illustrator, Editor, etc.)
CREATE TABLE IF NOT EXISTS roles (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;

INSERT OR IGNORE INTO roles (name) VALUES
    ('Author'),
    ('Translator'),
    ('Illustrator'),
    ('Cover Artist'),
    ('Editor'),
    ('Introduction Writer');

-- Book-level contributors (authors, editors of collected works, etc.)
CREATE TABLE IF NOT EXISTS book_contributors (
    book_id INTEGER NOT NULL,
    person_id INTEGER NOT NULL,
    role_id INTEGER NOT NULL,
    ordinal INTEGER,
    PRIMARY KEY (book_id, person_id, role_id),
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (person_id) REFERENCES persons(id) ON DELETE RESTRICT,
    FOREIGN KEY (role_id) REFERENCES roles(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_book_contributors_person_id ON book_contributors(person_id);
CREATE INDEX IF NOT EXISTS idx_book_contributors_book_id ON book_contributors(book_id);

-- Format lookup table (physical / commercial formats)
CREATE TABLE IF NOT EXISTS formats (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;

INSERT OR IGNORE INTO formats (name) VALUES
    ('Paperback'),
    ('Hardcover'),
    ('eBook');

-- Specific publication / edition of a work
CREATE TABLE IF NOT EXISTS editions (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL,
    format_id INTEGER NOT NULL,
    language TEXT NOT NULL,
    isbn TEXT UNIQUE,
    cover_filepath TEXT NOT NULL,
    cover_name TEXT,
    edition_name TEXT,
    edition_notes TEXT,
    publication_date TEXT,
    page_count INTEGER,
    listed INTEGER DEFAULT 1 CHECK (listed IN (0,1)),
    delisted_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (format_id) REFERENCES formats(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_editions_book_id ON editions(book_id);
CREATE INDEX IF NOT EXISTS idx_editions_isbn ON editions(isbn) WHERE isbn IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_editions_listed ON editions(listed);
CREATE INDEX IF NOT EXISTS idx_editions_language ON editions(language);

-- Edition-level contributors (translators, cover artists, etc.)
CREATE TABLE IF NOT EXISTS edition_contributors (
    edition_id INTEGER NOT NULL,
    person_id INTEGER NOT NULL,
    role_id INTEGER NOT NULL,
    ordinal INTEGER,
    PRIMARY KEY (edition_id, person_id, role_id),
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE CASCADE,
    FOREIGN KEY (person_id) REFERENCES persons(id) ON DELETE RESTRICT,
    FOREIGN KEY (role_id) REFERENCES roles(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_edition_contributors_person_id ON edition_contributors(person_id);
CREATE INDEX IF NOT EXISTS idx_edition_contributors_edition_id ON edition_contributors(edition_id);

-- Prices for editions in different currencies
CREATE TABLE IF NOT EXISTS edition_prices (
    edition_id INTEGER NOT NULL,
    currency TEXT NOT NULL,
    price INTEGER NOT NULL,
    PRIMARY KEY (edition_id, currency),
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_edition_prices_edition_id ON edition_prices(edition_id);

-- File format lookup table (delivery artifacts)
CREATE TABLE IF NOT EXISTS file_formats (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;

INSERT OR IGNORE INTO file_formats (name) VALUES
    ('epub'),
    ('kepub'),
    ('azw3'),
    ('pdf'),
    ('sample');

-- Concrete downloadable files for an edition
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    edition_id INTEGER NOT NULL,
    file_format_id INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    file_size INTEGER,
    checksum TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE CASCADE,
    FOREIGN KEY (file_format_id) REFERENCES file_formats(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_files_edition_id ON files(edition_id);

-- Categories / genres
CREATE TABLE IF NOT EXISTS categories (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;

INSERT OR IGNORE INTO categories (name) VALUES
    ('Poetry'),
    ('Fiction'),
    ('Sci-fi'),
    ('Non-fiction');

CREATE TABLE IF NOT EXISTS book_categories (
    book_id INTEGER NOT NULL,
    category_id INTEGER NOT NULL,
    PRIMARY KEY (book_id, category_id),
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_book_categories_category_id ON book_categories(category_id);
CREATE INDEX IF NOT EXISTS idx_book_categories_book_id ON book_categories(book_id);

-- Orders
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY,
    stripe_session_id TEXT UNIQUE,
    email TEXT,
    paid INTEGER DEFAULT 0 CHECK (paid IN (0,1)),
    paid_at TEXT,
    total_amount INTEGER,
    currency TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_orders_stripe_session ON orders(stripe_session_id);
CREATE INDEX IF NOT EXISTS idx_orders_email ON orders(email) WHERE email IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_orders_paid ON orders(paid);

-- Items in an order (editions purchased)
CREATE TABLE IF NOT EXISTS order_items (
    id INTEGER PRIMARY KEY,
    order_id INTEGER NOT NULL,
    edition_id INTEGER NOT NULL,
    quantity INTEGER NOT NULL,
    price_at_purchase INTEGER,
    currency_at_purchase TEXT,
    FOREIGN KEY (order_id) REFERENCES orders(id) ON DELETE CASCADE,
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_order_items_order_id ON order_items(order_id);
CREATE INDEX IF NOT EXISTS idx_order_items_edition_id ON order_items(edition_id);

-- Helpful views for common queries
-- View: Editions with localized content (no fallbacks)
CREATE VIEW IF NOT EXISTS editions_catalog AS
SELECT
    e.id as edition_id,
    e.isbn,
    e.cover,
    e.edition_name,
    e.edition_notes,
    e.publication_date,
    e.page_count,
    e.language,
    e.listed,
    b.id as book_id,
    b.slug as book_slug,
    b.original_language,
    b.original_publication_year,
    bl.title,
    bl.subtitle,
    bl.description,
    f.name as format,
    -- Primary author (ordinal = 1 or lowest)
    (SELECT pl.name
     FROM book_contributors bc
     JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = e.language
     JOIN roles r ON bc.role_id = r.id
     WHERE bc.book_id = b.id AND r.name = 'Author'
     ORDER BY bc.ordinal ASC NULLS LAST
     LIMIT 1
    ) as author_name,
    (SELECT pl.bio
     FROM book_contributors bc
     JOIN person_localizations pl ON pl.person_id = bc.person_id AND pl.language = e.language
     JOIN roles r ON bc.role_id = r.id
     WHERE bc.book_id = b.id AND r.name = 'Author'
     ORDER BY bc.ordinal ASC NULLS LAST
     LIMIT 1
    ) as author_bio,
    -- Translator if exists
    (SELECT pl.name
     FROM edition_contributors ec
     JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = e.language
     JOIN roles r ON ec.role_id = r.id
     WHERE ec.edition_id = e.id AND r.name = 'Translator'
     ORDER BY ec.ordinal ASC NULLS LAST
     LIMIT 1
    ) as translator_name
FROM editions e
JOIN books b ON e.book_id = b.id
JOIN formats f ON e.format_id = f.id
JOIN book_localizations bl ON bl.book_id = b.id AND bl.language = e.language;

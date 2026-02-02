-- Authors are independent entities (canonical form)
CREATE TABLE IF NOT EXISTS authors (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,            -- Canonical name (e.g., "Molière")
    bio TEXT
) STRICT;

-- The abstract "work" - the book as a concept
CREATE TABLE IF NOT EXISTS books (
    id INTEGER PRIMARY KEY,
    author_id INTEGER NOT NULL,
    slug TEXT NOT NULL UNIQUE,     -- Canonical slug (e.g., "moliere-don-juan")
    year_published INTEGER,
    FOREIGN KEY (author_id) REFERENCES authors(id) ON DELETE RESTRICT
) STRICT;

-- Format lookup table (physical / commercial formats)
CREATE TABLE IF NOT EXISTS formats (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE      -- Paperback, Hardcover, eBook
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
    title TEXT NOT NULL,           -- Localized title
    author_name TEXT,              -- Localized author name (fallback to authors.name)
    isbn TEXT UNIQUE,
    price INTEGER NOT NULL,        -- Smallest currency unit
    cover TEXT NOT NULL,
    description TEXT,
    edition_name TEXT,             -- "Revised", "Anniversary"
    translator TEXT,
    language TEXT,                 -- ISO 639-3 + script
    publication_date TEXT,         -- ISO date
    page_count INTEGER,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (format_id) REFERENCES formats(id) ON DELETE RESTRICT
) STRICT;

-- File format lookup table (delivery artifacts)
CREATE TABLE IF NOT EXISTS file_formats (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE      -- epub, kepub, azw3, pdf
) STRICT;

INSERT OR IGNORE INTO file_formats (name) VALUES
    ('epub'),
    ('kepub'),
    ('azw3'),
    ('pdf');

-- Concrete downloadable files for an edition
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    edition_id INTEGER NOT NULL,
    file_format_id INTEGER NOT NULL,
    file_path TEXT NOT NULL,       -- Path on disk
    file_size INTEGER,             -- Optional: bytes
    checksum TEXT,                 -- Optional: integrity hash
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE CASCADE,
    FOREIGN KEY (file_format_id) REFERENCES file_formats(id) ON DELETE RESTRICT,
    UNIQUE (edition_id, file_format_id)
) STRICT;

-- Categories / genres
CREATE TABLE IF NOT EXISTS categories (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE IF NOT EXISTS book_categories (
    book_id INTEGER NOT NULL,
    category_id INTEGER NOT NULL,
    PRIMARY KEY (book_id, category_id),
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE CASCADE
) STRICT;

-- Orders
-- paid: NULL = pending, 1 = paid, 0 = failed
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY,
    stripe_session_id TEXT UNIQUE,
    email TEXT,
    paid INTEGER CHECK (paid IN (0,1) OR paid IS NULL),
    paid_at TEXT,
    total_amount INTEGER,
    currency TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE INDEX IF NOT EXISTS idx_orders_stripe_session
    ON orders (stripe_session_id);

-- Items in an order (editions purchased)
CREATE TABLE IF NOT EXISTS order_items (
    id INTEGER PRIMARY KEY,
    order_id INTEGER NOT NULL,
    edition_id INTEGER NOT NULL,
    quantity INTEGER NOT NULL,
    price_at_purchase INTEGER,
    FOREIGN KEY (order_id) REFERENCES orders(id) ON DELETE CASCADE,
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_order_items_order_id
    ON order_items (order_id);

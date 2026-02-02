-- Authors are independent entities (canonical form)
CREATE TABLE IF NOT EXISTS authors (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,            -- Canonical name (e.g., "Molière")
    bio TEXT
) STRICT;

-- The abstract "work" - the book as a concept (minimal identity)
CREATE TABLE IF NOT EXISTS books (
    id INTEGER PRIMARY KEY,
    author_id INTEGER NOT NULL,
    slug TEXT NOT NULL UNIQUE,     -- Canonical slug for the book (e.g., "don-juan")
    year_published INTEGER,        -- Original publication year
    FOREIGN KEY (author_id) REFERENCES authors(id) ON DELETE RESTRICT
) STRICT;

-- Format lookup table (seeded values)
CREATE TABLE IF NOT EXISTS formats (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;

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
    file_path TEXT NOT NULL,       -- path to the edition's file on disk (used for downloads)
    description TEXT,              -- Book description
    edition_name TEXT,             -- "Revised", "Anniversary", "Deluxe"
    translator TEXT,               -- Nullable for translations
    language TEXT,                 -- ISO 639-3 language + ISO 15924 script tags (e.g., 'eng', 'fra', 'lzh-Hant')
    publication_date TEXT,         -- ISO date for this edition
    page_count INTEGER,
    FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE,
    FOREIGN KEY (format_id) REFERENCES formats(id) ON DELETE RESTRICT
) STRICT;

-- Categories/genres (many-to-many with books)
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

-- Orders table
-- paid is nullable: NULL = pending, 1 = paid, 0 = failed
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY,                          -- internal order id
    stripe_session_id TEXT UNIQUE,                   -- the Stripe Checkout session id (session_...)
    paid INTEGER CHECK (paid IN (0,1) OR paid IS NULL), -- NULL = pending, 1 = paid, 0 = failed
    paid_at TEXT,                                    -- RFC3339/ISO8601 timestamp when marked paid (nullable)
    total_amount INTEGER,                            -- optional: total in smallest currency unit (pence/cents)
    currency TEXT,                                   -- optional: 'GBP', 'USD', ...
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_orders_stripe_session ON orders (stripe_session_id);
CREATE INDEX IF NOT EXISTS idx_orders_client_ref ON orders (client_reference);

-- Items in an order: which editions and quantities
CREATE TABLE IF NOT EXISTS order_items (
    id INTEGER PRIMARY KEY,
    order_id INTEGER NOT NULL,
    edition_id INTEGER NOT NULL,
    quantity INTEGER NOT NULL,
    price_at_purchase INTEGER, -- price captured at time of checkout (smallest currency unit)
    FOREIGN KEY (order_id) REFERENCES orders(id) ON DELETE CASCADE,
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE RESTRICT
) STRICT;
CREATE INDEX IF NOT EXISTS idx_order_items_order_id ON order_items (order_id);

-- Download tokens: single-use timed tokens that map an order+edition to a download URL
CREATE TABLE IF NOT EXISTS download_tokens (
    id INTEGER PRIMARY KEY,
    order_id INTEGER NOT NULL,                -- which order this token belongs to
    edition_id INTEGER NOT NULL,              -- which edition/file this token grants access to
    token TEXT NOT NULL UNIQUE,               -- opaque token (use mint() to create)
    expires_at TEXT NOT NULL,                 -- RFC3339/ISO8601 expiration timestamp (store via chrono/sqlx)
    used INTEGER CHECK (used IN (0,1)) NOT NULL DEFAULT 0, -- 0 = unused, 1 = used
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')), -- RFC3339 timestamp
    FOREIGN KEY (order_id) REFERENCES orders(id) ON DELETE CASCADE,
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_download_tokens_token ON download_tokens (token);
CREATE INDEX IF NOT EXISTS idx_download_tokens_order ON download_tokens (order_id);

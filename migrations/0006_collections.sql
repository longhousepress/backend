-- Collections group related books (e.g. a trilogy). Grouping-only: no
-- pricing, no cover, no description — just a slug, localized titles, and
-- ordered members.

CREATE TABLE collections (
    id         INTEGER PRIMARY KEY,
    slug       TEXT    NOT NULL UNIQUE,
    created_at TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT
) STRICT;

CREATE TABLE collection_localizations (
    collection_id INTEGER NOT NULL,
    language      TEXT    NOT NULL REFERENCES languages(code),
    title         TEXT    NOT NULL,
    PRIMARY KEY (collection_id, language),
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE
) STRICT;

-- A book belongs to at most one collection (mirrors the single parent
-- `collection:` key in longhouse.yaml), hence book_id is the primary key.
CREATE TABLE collection_books (
    book_id       INTEGER NOT NULL PRIMARY KEY,
    collection_id INTEGER NOT NULL,
    ordinal       INTEGER NOT NULL,
    FOREIGN KEY (book_id)       REFERENCES books(id)       ON DELETE CASCADE,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_collection_books_collection_id ON collection_books(collection_id);

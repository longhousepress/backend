-- Make localizations a first-class citizen.
--
-- Introduces a curated languages lookup table and replaces ad-hoc per-table
-- name/title columns with uniform language-keyed localization tables:
--
--   languages              — ISO 639-3 lookup; the inserter hard-errors on
--                            unknown codes, so new languages must be added here.
--   edition_localizations  — per-edition, per-language marketing copy and listing
--                            state; replaces book_localizations (which was keyed
--                            on book, not edition) and the defunct storefronts
--                            indirection.
--   category_localizations — localized display names; name column removed from
--                            categories accordingly.
--   role_localizations     — localized display names; name column removed from
--                            roles accordingly.
--   orders.language        — records the display language the buyer used at
--                            checkout, mirroring how currency is stored.
--
-- Data-migration notes:
--   book_categories        — ON DELETE CASCADE to categories; rows are saved and
--                            restored around the categories rebuild.
--   edition_contributors / book_contributors — ON DELETE RESTRICT to roles; both
--                            tables are saved, dropped, roles rebuilt, recreated.

-- ── Languages ────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS languages (
    code TEXT NOT NULL PRIMARY KEY
) STRICT;

INSERT OR IGNORE INTO languages (code) VALUES ('eng');
INSERT OR IGNORE INTO languages (code) VALUES ('bul');
INSERT OR IGNORE INTO languages (code) VALUES ('kor');

-- ── Edition localizations ────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS edition_localizations (
    edition_id        INTEGER NOT NULL,
    language          TEXT    NOT NULL REFERENCES languages(code),
    listed            INTEGER NOT NULL DEFAULT 1 CHECK (listed IN (0, 1)),
    title             TEXT    NOT NULL,
    subtitle          TEXT,
    short_description TEXT    NOT NULL,
    description       TEXT,
    PRIMARY KEY (edition_id, language),
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_edition_localizations_edition_id ON edition_localizations(edition_id);
CREATE INDEX IF NOT EXISTS idx_edition_localizations_language   ON edition_localizations(language);

-- Drop old per-book localization and any defunct storefront tables.
DROP TABLE IF EXISTS edition_storefronts;
DROP TABLE IF EXISTS storefronts;
DROP TABLE IF EXISTS book_localizations;

-- ── Editions cleanup ─────────────────────────────────────────────────────────

-- Listing is now controlled per language via edition_localizations.listed.
DROP VIEW IF EXISTS editions_catalog;
DROP VIEW IF EXISTS edition_contributor_roles;
DROP INDEX IF EXISTS idx_editions_listed;

ALTER TABLE editions DROP COLUMN listed;
ALTER TABLE editions DROP COLUMN delisted_at;

-- ── Category localizations ───────────────────────────────────────────────────

CREATE TABLE category_localizations (
    category_id INTEGER NOT NULL,
    language    TEXT    NOT NULL REFERENCES languages(code),
    name        TEXT    NOT NULL,
    PRIMARY KEY (category_id, language),
    FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_category_localizations_category_id ON category_localizations(category_id);
CREATE INDEX idx_category_localizations_language     ON category_localizations(language);

-- ── Role localizations ───────────────────────────────────────────────────────

CREATE TABLE role_localizations (
    role_id  INTEGER NOT NULL,
    language TEXT    NOT NULL REFERENCES languages(code),
    name     TEXT    NOT NULL,
    PRIMARY KEY (role_id, language),
    FOREIGN KEY (role_id) REFERENCES roles(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_role_localizations_role_id  ON role_localizations(role_id);
CREATE INDEX idx_role_localizations_language ON role_localizations(language);

-- ── Rebuild categories without name column ───────────────────────────────────

-- book_categories ON DELETE CASCADE means DROP TABLE categories would lose the
-- join-table rows. Save them first; category_localizations is empty so its
-- cascade is a no-op.
CREATE TABLE book_categories_saved AS SELECT book_id, category_id FROM book_categories;
CREATE TABLE categories_new (
    id INTEGER PRIMARY KEY
) STRICT;
INSERT INTO categories_new SELECT id FROM categories;
DROP TABLE categories;
ALTER TABLE categories_new RENAME TO categories;
INSERT INTO book_categories SELECT book_id, category_id FROM book_categories_saved;
DROP TABLE book_categories_saved;

-- ── Rebuild roles without name column ────────────────────────────────────────

-- edition_contributors and book_contributors both have ON DELETE RESTRICT to
-- roles, so dropping roles would fire the RESTRICT. Save both tables, drop them,
-- rebuild roles, then recreate. role_localizations is empty so its cascade is
-- a no-op.
CREATE TABLE edition_contributors_saved AS
    SELECT edition_id, person_id, role_id, ordinal FROM edition_contributors;
CREATE TABLE book_contributors_saved AS
    SELECT book_id, person_id, role_id, ordinal FROM book_contributors;
DROP TABLE edition_contributors;
DROP TABLE book_contributors;
CREATE TABLE roles_new (
    id INTEGER PRIMARY KEY
) STRICT;
INSERT INTO roles_new SELECT id FROM roles;
DROP TABLE roles;
ALTER TABLE roles_new RENAME TO roles;
CREATE TABLE edition_contributors (
    edition_id INTEGER NOT NULL,
    person_id  INTEGER NOT NULL,
    role_id    INTEGER NOT NULL,
    ordinal    INTEGER,
    PRIMARY KEY (edition_id, person_id, role_id),
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE CASCADE,
    FOREIGN KEY (person_id)  REFERENCES persons(id)  ON DELETE RESTRICT,
    FOREIGN KEY (role_id)    REFERENCES roles(id)    ON DELETE RESTRICT
) STRICT;
CREATE INDEX idx_edition_contributors_person_id  ON edition_contributors(person_id);
CREATE INDEX idx_edition_contributors_edition_id ON edition_contributors(edition_id);
INSERT INTO edition_contributors
    SELECT edition_id, person_id, role_id, ordinal FROM edition_contributors_saved;
DROP TABLE edition_contributors_saved;
CREATE TABLE book_contributors (
    book_id   INTEGER NOT NULL,
    person_id INTEGER NOT NULL,
    role_id   INTEGER NOT NULL,
    ordinal   INTEGER,
    PRIMARY KEY (book_id, person_id, role_id),
    FOREIGN KEY (book_id)   REFERENCES books(id)   ON DELETE CASCADE,
    FOREIGN KEY (person_id) REFERENCES persons(id)  ON DELETE RESTRICT,
    FOREIGN KEY (role_id)   REFERENCES roles(id)    ON DELETE RESTRICT
) STRICT;
CREATE INDEX idx_book_contributors_person_id ON book_contributors(person_id);
CREATE INDEX idx_book_contributors_book_id   ON book_contributors(book_id);
INSERT INTO book_contributors
    SELECT book_id, person_id, role_id, ordinal FROM book_contributors_saved;
DROP TABLE book_contributors_saved;

-- ── Seed localizations ───────────────────────────────────────────────────────

INSERT INTO category_localizations (category_id, language, name) VALUES
    (1, 'eng', 'Poetry'),
    (2, 'eng', 'Sci-fi'),
    (3, 'eng', 'Non-fiction'),
    (4, 'eng', 'Adventure'),
    (5, 'eng', 'Drama'),
    (6, 'eng', 'Fantasy'),
    (7, 'eng', 'Horror'),
    (8, 'eng', 'Mystery'),
    (9, 'eng', 'Shorts'),
    (1, 'bul', 'Поезия'),
    (2, 'bul', 'Научна фантастика'),
    (3, 'bul', 'Нехудожествена литература'),
    (4, 'bul', 'Приключения'),
    (5, 'bul', 'Драма'),
    (6, 'bul', 'Фентъзи'),
    (7, 'bul', 'Ужаси'),
    (8, 'bul', 'Мистерия'),
    (9, 'bul', 'Разкази'),
    (1, 'kor', '시'),
    (2, 'kor', '과학소설'),
    (3, 'kor', '논픽션'),
    (4, 'kor', '모험'),
    (5, 'kor', '드라마'),
    (6, 'kor', '판타지'),
    (7, 'kor', '공포'),
    (8, 'kor', '미스터리'),
    (9, 'kor', '단편');

INSERT INTO role_localizations (role_id, language, name) VALUES
    (1, 'eng', 'Author'),
    (2, 'eng', 'Translator'),
    (3, 'eng', 'Illustrator'),
    (4, 'eng', 'Cover Artist'),
    (5, 'eng', 'Editor'),
    (6, 'eng', 'Introduction Writer'),
    (1, 'bul', 'Автор'),
    (2, 'bul', 'Преводач'),
    (3, 'bul', 'Илюстратор'),
    (4, 'bul', 'Художник на корицата'),
    (5, 'bul', 'Редактор'),
    (6, 'bul', 'Автор на предговора'),
    (1, 'kor', '저자'),
    (2, 'kor', '번역가'),
    (3, 'kor', '삽화가'),
    (4, 'kor', '표지 삽화가'),
    (5, 'kor', '편집자'),
    (6, 'kor', '서문 작가');

-- ── Recreate edition_contributor_roles view ──────────────────────────────────

-- Uses role IDs directly now that roles.name has been dropped.
CREATE VIEW edition_contributor_roles AS
SELECT
    ec.edition_id,
    MAX(CASE WHEN ec.role_id = 2 THEN pl.name END) as translator_name,
    MAX(CASE WHEN ec.role_id = 4 THEN pl.name END) as cover_artist_name,
    MAX(CASE WHEN ec.role_id = 3 THEN pl.name END) as illustrator_name,
    MAX(CASE WHEN ec.role_id = 6 THEN pl.name END) as introduction_writer_name
FROM edition_contributors ec
JOIN editions e ON e.id = ec.edition_id
JOIN person_localizations pl ON pl.person_id = ec.person_id AND pl.language = e.language
WHERE ec.role_id IN (2, 3, 4, 6)
GROUP BY ec.edition_id;

-- ── Orders: capture checkout language ────────────────────────────────────────

ALTER TABLE orders ADD COLUMN language TEXT NOT NULL DEFAULT 'eng';

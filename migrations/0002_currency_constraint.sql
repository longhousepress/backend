-- SQLite requires a table rebuild to add a CHECK constraint.
-- No FK disabling needed: nothing references edition_prices.
CREATE TABLE edition_prices_new (
    edition_id INTEGER NOT NULL,
    currency TEXT NOT NULL CHECK (currency IN ('USD', 'EUR', 'GBP', 'KRW')),
    price INTEGER NOT NULL,
    PRIMARY KEY (edition_id, currency),
    FOREIGN KEY (edition_id) REFERENCES editions(id) ON DELETE CASCADE
) STRICT;

INSERT INTO edition_prices_new SELECT * FROM edition_prices;

DROP TABLE edition_prices;

ALTER TABLE edition_prices_new RENAME TO edition_prices;

CREATE INDEX idx_edition_prices_edition_id ON edition_prices(edition_id);

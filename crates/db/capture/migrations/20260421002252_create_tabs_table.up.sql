-- Add up migration script here

CREATE TABLE tabs (
    tab_id           INTEGER PRIMARY KEY,
    url              TEXT    NOT NULL,
    tab_title        TEXT    NOT NULL,
    domain           TEXT    NOT NULL,

    captured_at      TEXT    NOT NULL,  -- ISO-8601; from TabCapture
    extractor        TEXT    NOT NULL,

    content          TEXT    NOT NULL,  -- JSON blob (ExtractedContent)
    extraction_ok    INTEGER NOT NULL,  -- SQLite has no BOOLEAN
    extraction_error TEXT,

    last_seen_at     TEXT    NOT NULL,  -- updated on every upsert
    created_at       TEXT    NOT NULL   -- set once on first insert
);

-- last_seen_at is the pruning dimension; index it.
CREATE INDEX idx_tabs_last_seen_at ON tabs(last_seen_at DESC);

-- domain is useful for scheduler graph grouping.
CREATE INDEX idx_tabs_domain ON tabs(domain);


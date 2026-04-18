-- Add up migration script here
-- migrations/0001_capture_sessions.sql
CREATE TABLE IF NOT EXISTS capture_sessions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT    NOT NULL UNIQUE,
    captured_at     TEXT    NOT NULL,
    extension_version TEXT  NOT NULL,
    total_open_tabs INTEGER NOT NULL,
    -- TabCapture[] and SkippedTab[] stored as JSON; nested structs have
    -- no stable query predicate beyond domain/session so normalising
    -- adds joins with no benefit in this access pattern.
    captures        TEXT    NOT NULL DEFAULT '[]',   -- JSON
    skipped         TEXT    NOT NULL DEFAULT '[]'    -- JSON
);

CREATE INDEX IF NOT EXISTS idx_capture_sessions_session_id
    ON capture_sessions (session_id);

CREATE INDEX IF NOT EXISTS idx_capture_sessions_captured_at
    ON capture_sessions (captured_at);

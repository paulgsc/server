-- Reverses 20260816000500_add_subject_to_sessions.up.sql: rebuilds
-- `sessions` back to the exact shape 20260805000300_create_sessions.up.sql
-- created, dropping `subject_id`. The backfilled values are discarded, not
-- preserved — there is no column left to hold them once this runs, and
-- rolling the up migration forward again re-derives the same singleton value
-- from `subject.rs`, so nothing here is lossy in a way that matters.

CREATE TABLE sessions_old (
    id                TEXT    PRIMARY KEY,
    name              TEXT    NOT NULL,
    status            TEXT    NOT NULL,
    layout_mode       TEXT    NOT NULL,
    total_duration_ms INTEGER NOT NULL,

    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    started_at        TEXT,
    completed_at      TEXT,
    final_elapsed_ms  INTEGER,

    activities        TEXT    NOT NULL,
    scenes            TEXT    NOT NULL,
    layout            TEXT
);

INSERT INTO sessions_old (
    id, name, status, layout_mode, total_duration_ms,
    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
    activities, scenes, layout
)
SELECT
    id, name, status, layout_mode, total_duration_ms,
    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
    activities, scenes, layout
FROM sessions;

DROP TABLE sessions;
ALTER TABLE sessions_old RENAME TO sessions;

CREATE INDEX idx_sessions_started_at ON sessions(started_at);
CREATE INDEX idx_sessions_completed_at ON sessions(completed_at);
CREATE INDEX idx_sessions_status ON sessions(status, updated_at DESC);

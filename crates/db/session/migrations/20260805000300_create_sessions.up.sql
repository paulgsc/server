-- A transcription of `SessionRecord` from apps/www/src/lib/tenant/types.ts in
-- paulgsc/some-ui, not a redesign of it. The nudge needs to ask "has this
-- person studied today", and that answer lived in localStorage where no
-- server could read it.
--
-- The split between columns and JSON blobs is the one deliberate decision
-- here. `activities`, `scenes`, and `layout` are nested structures the server
-- never reasons about — modelling a layout tree in SQL would buy nothing and
-- cost a migration every time the client's shape moves. The scalars the
-- policy actually filters on are real columns, because those get indexed.
-- `capture_repo` already stores `ExtractedContent` the same way.

CREATE TABLE sessions (
    id                TEXT    PRIMARY KEY,   -- "session-<uuid>", minted server-side
    name              TEXT    NOT NULL,
    status            TEXT    NOT NULL,      -- draft | scheduled | active | paused | completed
    layout_mode       TEXT    NOT NULL,      -- basic | advanced
    total_duration_ms INTEGER NOT NULL,      -- derived from scenes; the server computes it

    created_at        TEXT    NOT NULL,      -- ISO-8601
    updated_at        TEXT    NOT NULL,      -- ISO-8601; editing only, never "studied"
    started_at        TEXT,                  -- ISO-8601; the nudge reads this
    completed_at      TEXT,                  -- ISO-8601; and this
    final_elapsed_ms  INTEGER,

    -- JSON TEXT. `layout` distinguishes three states the client cares about:
    -- SQL NULL is "absent" (the naive default layout applies), the four-byte
    -- string 'null' is an explicit null, and anything else is a tree.
    activities        TEXT    NOT NULL,
    scenes            TEXT    NOT NULL,
    layout            TEXT
);

-- "Did I study today" filters on exactly these two, and on nothing else.
CREATE INDEX idx_sessions_started_at ON sessions(started_at);
CREATE INDEX idx_sessions_completed_at ON sessions(completed_at);

-- Candidate ranking scans by status and breaks ties on updated_at.
CREATE INDEX idx_sessions_status ON sessions(status, updated_at DESC);

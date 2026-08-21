-- P1.1 (#259): every table the study policy reads is keyed by subject except
-- this one. `sessions` predates `file_host::subject` — see
-- 20260805000300_create_sessions.up.sql's own comment, which only had to
-- reason about scenes and layout, not ownership — so it is retrofitted here
-- rather than designed in from the start.
--
-- This deployment has exactly one subject
-- (`file_host::subject::SINGLETON_SUBJECT`, `'subject-local'`), so the
-- backfill has one right answer and no ambiguity: every row that exists
-- before this migration was written by that subject, because nothing else
-- could have written it.
--
-- `ALTER TABLE ... ADD COLUMN` cannot add a `NOT NULL` column without a
-- default, and leaving a permanent default in place is exactly the footgun
-- this epic (#252) exists to remove — a future row that forgot to name its
-- owner would silently become the singleton's. So the table is rebuilt
-- instead: a fresh `sessions_new` declares `subject_id` `NOT NULL` with no
-- default at all, an `INSERT ... SELECT` names the backfill value exactly
-- once for every existing row, and the rebuilt table is swapped in for the
-- original. This is the `CREATE TABLE ... + INSERT SELECT + DROP + RENAME`
-- shape rather than the 12-step `PRAGMA legacy_alter_table` dance — no prior
-- migration in this repo needed either, so there is no established idiom to
-- match, and this one reads top to bottom.
--
-- `SessionRepository`'s queries are untouched here on purpose (see #259's
-- own out-of-scope note): they neither read nor write `subject_id` yet, so a
-- write through `upsert` would fail loudly on the new `NOT NULL` constraint
-- the moment this lands. That is acceptable only because P1 (#295) is not a
-- deployable increment by itself — #260 (SUB2) is next in the landing order
-- and threads a real subject through every query before anything ships.
CREATE TABLE sessions_new (
    id                TEXT    PRIMARY KEY,   -- "session-<uuid>", minted server-side
    subject_id        TEXT    NOT NULL,      -- owner; singleton-backfilled, never defaulted
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

INSERT INTO sessions_new (
    id, subject_id, name, status, layout_mode, total_duration_ms,
    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
    activities, scenes, layout
)
SELECT
    id, 'subject-local', name, status, layout_mode, total_duration_ms,
    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
    activities, scenes, layout
FROM sessions;

DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;

-- "Did I study today" filters on exactly these two, unchanged from the
-- original migration. Neither gains `subject_id` as a leading column here:
-- `SessionRepository::touched_between` does not filter by subject yet (that
-- is #260's job, same as every other query on this table), so a composite
-- index led by `subject_id` would not match its `WHERE` clause today and
-- would just be dead weight until that story lands and the query changes
-- with it.
CREATE INDEX idx_sessions_started_at ON sessions(started_at);
CREATE INDEX idx_sessions_completed_at ON sessions(completed_at);

-- Candidate ranking scans by status and breaks ties on updated_at — see the
-- original migration's comment for that scan's justification. It is about to
-- become a per-subject scan once #260 lands, so it is rebuilt now with
-- `subject_id` leading rather than appended: a lookup for one subject's
-- candidates should not have to skip over every other subject's rows first.
CREATE INDEX idx_sessions_status ON sessions(subject_id, status, updated_at DESC);

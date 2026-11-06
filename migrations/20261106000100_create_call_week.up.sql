-- The weekly call's weeks, kept on home (STUDIO ST36's second slice:
-- paulgsc/some-ui#2635, the owner's decisions of 2026-11-06). The weekly
-- routine writes each week to `paulgsc/testing` under `call/`, and that
-- repository keeps only the last few (`call/index.json`'s `keep`). Home keeps
-- every one: `file_host`'s `call_weeks` worker reads the index on a timer and
-- stores each week it does not hold yet, and the routine lets a week leave
-- the repository only once home's `call` index in the mailbox lists it.
-- `call_week_repo` is the one module that writes this table.
--
-- Not subject-scoped: the call says what merged in two public repositories,
-- read from their public history. No column names a person, and nothing here
-- is read per subject.
--
-- APPEND-ONLY: the triggers abort every `UPDATE` and `DELETE`. A week is
-- kept once, by its first day, as the repository first served it; a later
-- edit there is not taken, and the routine sees it by the SHA-256 in home's
-- index (paulgsc/some-ui `packages/video-render/CALL-ROUTINE.md`).
--
-- Columns:
--
--   week_from -- the week's first day, `YYYY-MM-DD`: its file's name.
--   week_to   -- its last day, both included.
--   sha256    -- lowercase hex SHA-256 of `body`.
--   size      -- `body`'s length in bytes.
--   body      -- the week's file, exactly as the repository served it.
--   taken_at  -- ISO-8601 UTC, when home stored it.
CREATE TABLE call_week (
    week_from TEXT    NOT NULL PRIMARY KEY CHECK (length(week_from) = 10),
    week_to   TEXT    NOT NULL CHECK (length(week_to) = 10 AND week_to >= week_from),
    sha256    TEXT    NOT NULL CHECK (length(sha256) = 64),
    size      INTEGER NOT NULL CHECK (size = length(body)),
    body      BLOB    NOT NULL,
    taken_at  TEXT    NOT NULL
);

CREATE TRIGGER call_week_no_update
BEFORE UPDATE ON call_week
BEGIN
    SELECT RAISE(ABORT, 'call_week is append-only: a kept week is never rewritten');
END;

CREATE TRIGGER call_week_no_delete
BEFORE DELETE ON call_week
BEGIN
    SELECT RAISE(ABORT, 'call_week is append-only: a kept week is never deleted');
END;

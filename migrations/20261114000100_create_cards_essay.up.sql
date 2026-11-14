-- The cards essays, kept on home (CARDS7: paulgsc/server#854,
-- paulgsc/some-ui#2979, call C327). A session writes each essay's cut list to
-- `paulgsc/testing` under `cards/<key>.json` and lists it in
-- `cards/index.json`; `file_host`'s `cards_essays` worker reads the index on a
-- timer and stores each essay home does not hold yet, then gives it to the
-- render worker, which makes it into a silent video of cards.
-- `cards_essay_repo` is the one module that writes this table.
--
-- Not subject-scoped: an essay explains public repositories at stamped
-- commits, written by a session, with no take and nothing the owner said. No
-- column names a person, and nothing here is read per subject.
--
-- APPEND-ONLY: the triggers abort every `UPDATE` and `DELETE`. An essay is
-- kept once, by its key, as the repository first served it; a later edit
-- there is not taken (the routine never rewrites one: the same subject checked
-- again is a new essay, under a new day).
--
-- Columns:
--
--   essay_key -- `<slug>-<YYYY-MM-DD>`, the stamp's day last: its file's name.
--                The slug's shape is checked by the worker
--                (`cards_essays::pass::is_key`); the table holds the day's.
--   day       -- the key's day, `YYYY-MM-DD`: the order essays render in.
--   sha256    -- lowercase hex SHA-256 of `body`.
--   size      -- `body`'s length in bytes.
--   body      -- the cut list, exactly as the repository served it.
--   taken_at  -- ISO-8601 UTC, when home stored it.
CREATE TABLE cards_essay (
    essay_key TEXT    NOT NULL PRIMARY KEY CHECK (
        length(essay_key) BETWEEN 12 AND 128
        AND essay_key GLOB '[a-z0-9]*-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
        AND essay_key NOT GLOB '*[^a-z0-9-]*'
        AND substr(essay_key, -10) = day
    ),
    day       TEXT    NOT NULL CHECK (day GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    sha256    TEXT    NOT NULL CHECK (length(sha256) = 64),
    size      INTEGER NOT NULL CHECK (size = length(body)),
    body      BLOB    NOT NULL,
    taken_at  TEXT    NOT NULL
);

CREATE TRIGGER cards_essay_no_update
BEFORE UPDATE ON cards_essay
BEGIN
    SELECT RAISE(ABORT, 'cards_essay is append-only: a kept essay is never rewritten');
END;

CREATE TRIGGER cards_essay_no_delete
BEFORE DELETE ON cards_essay
BEGIN
    SELECT RAISE(ABORT, 'cards_essay is append-only: a kept essay is never deleted');
END;

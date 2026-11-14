-- A newer revision of a week of the call home holds (paulgsc/server#879,
-- the owner's direction of 2026-11-14, paulgsc/some-ui#2918). `call_week`
-- keeps a week once, as first served. A week the routine re-stamps in
-- `paulgsc/testing` (template version 5, a long week as a series of parts)
-- was then never seen by home, which kept rendering the copy it first took
-- and was refused for good. So home takes a re-stamp as a new revision here,
-- **only while no part of the week has a video**: the `call_weeks` pass asks
-- for each held week with none, and appends what it serves when its bytes
-- differ from the newest revision home keeps. Once any part has a video, the
-- week is frozen at the revision it was rendered from, as before.
-- `call_week_repo` is the one module that writes this table.
--
-- **Nothing is rewritten** (paulgsc/some-ui CLAUDE.md, "Never lose the
-- record"): `call_week`'s row is revision 1 and stays as it is; each later
-- revision is a row here. The newest of a week is the one home serves, to
-- the render worker and in the mailbox's `call` index.
--
-- Not subject-scoped, as `call_week` is not. No column names a person.
--
-- APPEND-ONLY: the triggers abort every `UPDATE` and `DELETE`. An INSERT is
-- refused for a week home does not hold, for a revision that is not the
-- next, and for a week a part of which has a link or a receipt (a video was
-- made of it: the pass also refuses while a part's `call.mp4` is on the
-- drive, which this table cannot see).
--
-- Columns:
--
--   week_from -- the week, as `call_week.week_from`.
--   revision  -- 2 for the first re-stamp, then 3, ...; `call_week`'s row is 1.
--   week_to   -- its last day, both included, as this revision says.
--   sha256    -- lowercase hex SHA-256 of `body`.
--   size      -- `body`'s length in bytes.
--   body      -- the week's file, exactly as the repository served it.
--   taken_at  -- ISO-8601 UTC, when home stored it.
CREATE TABLE call_week_revision (
    week_from TEXT    NOT NULL REFERENCES call_week (week_from),
    revision  INTEGER NOT NULL CHECK (revision >= 2),
    week_to   TEXT    NOT NULL CHECK (week_to GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]' AND week_to >= week_from),
    sha256    TEXT    NOT NULL CHECK (length(sha256) = 64),
    size      INTEGER NOT NULL CHECK (size = length(body)),
    body      BLOB    NOT NULL,
    taken_at  TEXT    NOT NULL,
    PRIMARY KEY (week_from, revision)
);

CREATE TRIGGER call_week_revision_in_order
BEFORE INSERT ON call_week_revision
BEGIN
    SELECT RAISE(ABORT, 'call_week_revision: home holds no such week')
    WHERE NOT EXISTS (SELECT 1 FROM call_week WHERE week_from = NEW.week_from);
    SELECT RAISE(ABORT, 'call_week_revision: a revision is the next after the newest')
    WHERE NEW.revision <> 1 + COALESCE((SELECT MAX(revision) FROM call_week_revision WHERE week_from = NEW.week_from), 1);
    SELECT RAISE(ABORT, 'call_week_revision: a part of the week has a video, so the week is frozen')
    WHERE EXISTS (SELECT 1 FROM call_part_link WHERE week_from = NEW.week_from)
       OR EXISTS (SELECT 1 FROM call_part_deletion WHERE week_from = NEW.week_from);
END;

CREATE TRIGGER call_week_revision_no_update
BEFORE UPDATE ON call_week_revision
BEGIN
    SELECT RAISE(ABORT, 'call_week_revision is append-only: a kept revision is never rewritten');
END;

CREATE TRIGGER call_week_revision_no_delete
BEFORE DELETE ON call_week_revision
BEGIN
    SELECT RAISE(ABORT, 'call_week_revision is append-only: a kept revision is never deleted');
END;

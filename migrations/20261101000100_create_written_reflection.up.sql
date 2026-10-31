-- Written reflections (paulgsc/some-ui#2178, HF3): each typed line the
-- owner's phone keeps about a study session (paulgsc/some-ui
-- `apps/www/src/lib/session-reflection`), sent home by the phone's drain.
-- Home is the owner's own store (paulgsc/some-ui#2174, decision D1), so it is
-- the record: the phone keeps a few for when it is away from home, and lets
-- one go only once home holds it.
--
-- It is a record, not a convenience copy, so it is APPEND-ONLY. A
-- reflection, once stored, is never updated (the trigger below aborts an
-- UPDATE) or replaced: `reflection_repo` refuses a second, different
-- reflection under an id it holds. The only deletes are account deletion's.
--
-- Written only: a spoken reflection goes home as a voice note (`voice_note`),
-- and the one-tap answers stay on the phone. What the reflections routine
-- reads of each (its text, its session and when) is filed for the Jobs
-- mailbox's `reflection` folder as it arrives (`mailbox_outgoing`,
-- docs/identity.md invariant 19); nothing else of it leaves home.
--
-- Columns of `written_reflection`:
--
--   subject_id    -- whose record.
--   reflection_id -- the phone's id for the reflection (a UUID today).
--   session_id    -- the phone's id for the session it is about.
--   at            -- when it was written, as ISO-8601 UTC to the
--                    millisecond.
--   at_ms         -- how far into the session it was written, in
--                    milliseconds; NULL when written after it ended.
--   text          -- the typed line, as typed. It is the owner's own words,
--                    so it never reaches a log line.
--   written_at    -- ISO-8601 UTC, when it reached home.
CREATE TABLE written_reflection (
    subject_id    TEXT NOT NULL,
    reflection_id TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    at            TEXT NOT NULL,
    at_ms         INTEGER,
    text          TEXT NOT NULL,
    written_at    TEXT NOT NULL,
    PRIMARY KEY (subject_id, reflection_id)
);

CREATE TRIGGER written_reflection_is_append_only
BEFORE UPDATE ON written_reflection
BEGIN
    SELECT RAISE(ABORT, 'written_reflection is append-only: write a new reflection');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER written_reflection_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM written_reflection WHERE subject_id = OLD.subject_id;
END;

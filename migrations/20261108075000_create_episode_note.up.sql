-- Episode notes (paulgsc/some-ui#2745, SFEED6): a note the owner left at a
-- moment of one of a feed's episodes on the phone's Feedback page
-- (paulgsc/some-ui `apps/www/src/lib/episode-feedback`), picked from a fixed
-- set of categories, sent home by the phone's drain. Home is the owner's own
-- store, so it is the record.
--
-- It is a record, not a convenience copy, so it is APPEND-ONLY: a note, once
-- stored, is never updated (the trigger below aborts an UPDATE) or replaced:
-- `episode_note_repo` refuses a second, different note under an id it holds.
-- There is no delete and no switch to turn one off (the owner's answer on
-- paulgsc/some-ui#2749); the only deletes are account deletion's.
--
-- What the routine reads of each (all of it, but `stored_at`) is filed for
-- the Jobs mailbox's `feedback` folder as it arrives (`mailbox_outgoing`,
-- docs/identity.md invariant 19). A spoken note is a library take, a voice
-- note home keeps (`voice_note`, invariant 23), named here by its id only:
-- what home heard in it reaches the routine as that take's transcript, in the
-- mailbox's `transcript` folder, and nothing of its audio comes near here.
--
-- Columns of `episode_note`:
--
--   subject_id  -- whose record.
--   note_id     -- the phone's id for the note.
--   feed        -- the feed whose episode it is about: `call` or
--                  `playoff-delta` (`file_host::feeds::Feed`).
--   episode_key -- the episode's key, as home lists it (`2026-10-30`,
--                  `2026-SF-08`).
--   at_ms       -- the moment in the episode, in milliseconds from its start.
--   categories  -- the categories picked, a JSON array of names from the
--                  fixed set, each once, in the set's order, at least one;
--                  stored compact (`["shallow","voice"]`), which the trigger
--                  below holds it to.
--   take_id     -- the voice note id of the library take said for it, or
--                  NULL for a note with no spoken part.
--   written_at  -- when it was written, as ISO-8601 UTC to the millisecond.
--   stored_at   -- ISO-8601 UTC, when it reached home.
CREATE TABLE episode_note (
    subject_id  TEXT    NOT NULL,
    note_id     TEXT    NOT NULL,
    feed        TEXT    NOT NULL CHECK (feed IN ('call', 'playoff-delta')),
    episode_key TEXT    NOT NULL CHECK (length(episode_key) BETWEEN 1 AND 64),
    at_ms       INTEGER NOT NULL CHECK (at_ms >= 0),
    categories  TEXT    NOT NULL CHECK (json_valid(categories) AND json_type(categories) = 'array' AND json_array_length(categories) BETWEEN 1 AND 14),
    take_id     TEXT             CHECK (take_id IS NULL OR length(take_id) BETWEEN 1 AND 64),
    written_at  TEXT    NOT NULL,
    stored_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

-- `GET /feeds/:feed/notes`: one subject's notes on one feed, oldest first,
-- read as a range.
CREATE INDEX episode_note_by_feed ON episode_note (subject_id, feed, written_at, note_id);

-- The categories are the fixed set's, each once, in its order
-- (paulgsc/some-ui `episode-feedback/model.ts`, `CATEGORIES`): the stored
-- array must be exactly the set's names it holds, in the set's order, so an
-- unknown name, one twice or two out of order is refused. A CHECK cannot read
-- `json_each`, so a trigger holds it.
CREATE TRIGGER episode_note_categories_are_the_set
BEFORE INSERT ON episode_note
WHEN NEW.categories IS NOT (
    SELECT json_group_array(column2) FROM (
        SELECT column2 FROM (VALUES
            (1, 'shallow'), (2, 'technical'), (3, 'missedIntuition'), (4, 'unsupported'),
            (5, 'tooLong'), (6, 'tooShort'), (7, 'textClipped'), (8, 'slideShakes'),
            (9, 'overlap'), (10, 'hardToRead'), (11, 'pacing'), (12, 'voice'),
            (13, 'musicLoud'), (14, 'subtitlesOff'))
        WHERE column2 IN (SELECT value FROM json_each(NEW.categories))
        ORDER BY column1
    )
)
BEGIN
    SELECT RAISE(ABORT, 'episode_note.categories must be names from the fixed set, each once, in its order');
END;

CREATE TRIGGER episode_note_is_append_only
BEFORE UPDATE ON episode_note
BEGIN
    SELECT RAISE(ABORT, 'episode_note is append-only: write a new note');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `written_reflection` does.
CREATE TRIGGER episode_note_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM episode_note WHERE subject_id = OLD.subject_id;
END;

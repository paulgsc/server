-- Reverses 20261119000100_take_notes_on_every_feed.up.sql: rebuilds
-- `episode_note` back to the two feeds 20261108075000 allowed, the same way.
--
-- **It refuses rather than discards,** as
-- 20261028000100_add_game_bite_to_voice_note.down.sql does: every note is
-- copied, one on any other feed included, into a table whose CHECK refuses
-- it, so once a note on another feed is held this fails on that row and the
-- migration's transaction rolls back, leaving the table as it was.
--
-- The account trigger names `episode_note` in its body, so it goes first and
-- comes back last.
DROP TRIGGER episode_note_leaves_with_account;

CREATE TABLE episode_note_old (
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

INSERT INTO episode_note_old (subject_id, note_id, feed, episode_key, at_ms, categories, take_id, written_at, stored_at)
SELECT subject_id, note_id, feed, episode_key, at_ms, categories, take_id, written_at, stored_at FROM episode_note;

DROP TABLE episode_note;
ALTER TABLE episode_note_old RENAME TO episode_note;

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

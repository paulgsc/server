-- Notes on every feed's episodes (paulgsc/server#983, SFEED; the owner,
-- 2026-11-18: "the note is generic over the video"). 20261108075000 held
-- `episode_note.feed` to `call` and `playoff-delta`; a note is now taken on
-- an episode of any feed home makes videos of, by the feed being one
-- (`file_host::feeds::Feed::parse`, which `handlers::episode_notes` asks),
-- so the table holds `feed` to a feed's shape only, as `game_bite` holds
-- `scenario_id` to a shape and not to a set: a feed added to `Feed` takes
-- notes with no migration.
--
-- SQLite cannot alter a CHECK, so the table is rebuilt as
-- 20261028000100_add_game_bite_to_voice_note rebuilt `voice_note`: a fresh
-- `episode_note_new` with the wider CHECK, every row copied across
-- unchanged, the old table dropped and the new one renamed in. Only the
-- CHECK on `feed` widens; every column, the index, the categories and
-- append-only triggers and the account trigger are recreated word for word.
-- The account trigger names `episode_note` in its body, so it is dropped
-- first and recreated once the rebuilt table holds the name again.
--
-- `feed` -- the feed whose episode it is about, as the routes name it
--           (`call`, `cards-essay`, `interview-essay`, `playoff-delta`,
--           `report`, `rust-essay`): 1 to 32 lowercase letters, digits or
--           `-`.
DROP TRIGGER episode_note_leaves_with_account;

CREATE TABLE episode_note_new (
    subject_id  TEXT    NOT NULL,
    note_id     TEXT    NOT NULL,
    feed        TEXT    NOT NULL CHECK (length(feed) BETWEEN 1 AND 32 AND feed NOT GLOB '*[^a-z0-9-]*'),
    episode_key TEXT    NOT NULL CHECK (length(episode_key) BETWEEN 1 AND 64),
    at_ms       INTEGER NOT NULL CHECK (at_ms >= 0),
    categories  TEXT    NOT NULL CHECK (json_valid(categories) AND json_type(categories) = 'array' AND json_array_length(categories) BETWEEN 1 AND 14),
    take_id     TEXT             CHECK (take_id IS NULL OR length(take_id) BETWEEN 1 AND 64),
    written_at  TEXT    NOT NULL,
    stored_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

INSERT INTO episode_note_new (subject_id, note_id, feed, episode_key, at_ms, categories, take_id, written_at, stored_at)
SELECT subject_id, note_id, feed, episode_key, at_ms, categories, take_id, written_at, stored_at FROM episode_note;

DROP TABLE episode_note;
ALTER TABLE episode_note_new RENAME TO episode_note;

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

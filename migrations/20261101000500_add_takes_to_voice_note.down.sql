-- Reverses 20261101000500_add_takes_to_voice_note.up.sql: rebuilds
-- `voice_note` back to the five "said for"s 20261030000100 allowed, the same
-- way.
--
-- **It refuses rather than discards,** as 20261030000100's down does: every
-- note is copied, a `soundbite`, `lens` or `leetype` one included, into a
-- table whose CHECK refuses those, so once any such note is held this fails
-- on that row and the migration's transaction rolls back, leaving the table
-- as it was.
--
-- The account trigger names this table, so it goes first and comes back
-- last.
DROP TRIGGER voice_note_leaves_with_account;

CREATE TABLE voice_note_old (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene', 'game', 'studio')),
    of_ref       TEXT    NOT NULL,
    recorded_at  TEXT    NOT NULL,
    content_type TEXT    NOT NULL CHECK (content_type IN ('audio/webm', 'audio/ogg', 'audio/mp4')),
    size         INTEGER NOT NULL CHECK (size > 0),
    sha256       TEXT    NOT NULL,
    stored_at    TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

INSERT INTO voice_note_old (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
SELECT subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at FROM voice_note;

DROP TABLE voice_note;
ALTER TABLE voice_note_old RENAME TO voice_note;

CREATE TRIGGER voice_note_is_append_only
BEFORE UPDATE ON voice_note
BEGIN
    SELECT RAISE(ABORT, 'voice_note is append-only: a note is never rewritten');
END;

CREATE INDEX voice_note_by_stored_at ON voice_note (stored_at, subject_id, note_id);

CREATE TRIGGER voice_note_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note WHERE subject_id = OLD.subject_id;
    DELETE FROM voice_note_audio WHERE subject_id = OLD.subject_id;
END;

-- Reverses 20261028000100_add_game_bite_to_voice_note.up.sql: rebuilds
-- `voice_note` back to the three "said for"s 20261025000100 allowed, the same
-- way, and drops `game_bite`.
--
-- **It refuses rather than discards,** as the `sharing_choice` rebuilds'
-- downs do: every note is copied, a `game` one included, into a table whose
-- CHECK refuses `game`, so once any bite is held this fails on that row and
-- the migration's transaction rolls back, leaving both tables as they were.
-- `game_bite` is held to the same: its rows are copied into a table that
-- takes none before it is dropped, so a bite with no note (which `voice_repo`
-- never writes) refuses the rollback too.
--
-- The account triggers name these tables, so they go first, and
-- `voice_note`'s comes back last.
DROP TRIGGER game_bite_leaves_with_account;
DROP TRIGGER voice_note_leaves_with_account;

CREATE TABLE voice_note_old (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene')),
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

CREATE TABLE game_bite_must_be_empty (note_id TEXT CHECK (note_id IS NULL));
INSERT INTO game_bite_must_be_empty (note_id) SELECT note_id FROM game_bite;
DROP TABLE game_bite_must_be_empty;

DROP TRIGGER game_bite_is_append_only;
DROP TABLE game_bite;

CREATE TRIGGER voice_note_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note WHERE subject_id = OLD.subject_id;
    DELETE FROM voice_note_audio WHERE subject_id = OLD.subject_id;
END;

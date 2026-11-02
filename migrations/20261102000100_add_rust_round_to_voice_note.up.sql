-- Rust Rounds' voice notes are voice notes (paulgsc/some-ui#2237, RR6, epic
-- paulgsc/some-ui#2231), under HOME-FIRST (paulgsc/some-ui#2174, D1): each
-- goes home like any other, kept as `voice_note` and `voice_note_audio` keep
-- every note (append-only, kept for good, docs/identity.md invariant 23),
-- said for one new kind:
--
--   rust-round -- a note said on a Rust Rounds card; its `ref` names the
--                 card, and on a refine card the note it refines
--                 (`voice_repo::is_rust_round_ref`).
--
-- Nothing sits beside the note in this table's family: no side table, as for
-- `studio`. A new take is a new note under a new id; nothing is rewritten.
--
-- `voice_note` takes one more "said for". SQLite cannot alter a CHECK, so it
-- is rebuilt exactly as 20261101000500_add_takes_to_voice_note rebuilt it: a
-- fresh `voice_note_new` with the wider CHECK, every row copied across
-- unchanged, the old table dropped and the new one renamed in. Only the CHECK
-- on `of_kind` widens; every column, the append-only trigger, the
-- transcriber's index (`voice_note_by_stored_at`) and the account trigger are
-- recreated word for word. The account trigger names `voice_note` in its
-- body, so it is dropped first and recreated once the rebuilt table holds the
-- name again. `voice_note_audio`, `game_bite`, `voice_transcript`,
-- `voice_transcript_word`, `mailbox_transcript`, `studio_take_on_drive`,
-- `voice_note_set_aside` and the idea tables name a note by its id only and
-- are untouched.
DROP TRIGGER voice_note_leaves_with_account;

CREATE TABLE voice_note_new (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene', 'game', 'studio', 'soundbite', 'lens', 'leetype', 'rust-round')),
    of_ref       TEXT    NOT NULL,
    recorded_at  TEXT    NOT NULL,
    content_type TEXT    NOT NULL CHECK (content_type IN ('audio/webm', 'audio/ogg', 'audio/mp4')),
    size         INTEGER NOT NULL CHECK (size > 0),
    sha256       TEXT    NOT NULL,
    stored_at    TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

INSERT INTO voice_note_new (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
SELECT subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at FROM voice_note;

DROP TABLE voice_note;
ALTER TABLE voice_note_new RENAME TO voice_note;

CREATE TRIGGER voice_note_is_append_only
BEFORE UPDATE ON voice_note
BEGIN
    SELECT RAISE(ABORT, 'voice_note is append-only: a note is never rewritten');
END;

-- The worker takes the oldest notes first.
CREATE INDEX voice_note_by_stored_at ON voice_note (stored_at, subject_id, note_id);

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER voice_note_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note WHERE subject_id = OLD.subject_id;
    DELETE FROM voice_note_audio WHERE subject_id = OLD.subject_id;
END;

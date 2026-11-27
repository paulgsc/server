-- A spoken hunch is a voice note (paulgsc/server#1261, KPLAY's KP4;
-- paulgsc/some-ui#2853): the owner says, in English, what they want to say in
-- Korean, and home hears it as it hears every note, then finds the phrases
-- that do it (`POST /api/v1/phrases/lookup`). It goes home like any other,
-- kept as `voice_note` and `voice_note_audio` keep every note (append-only,
-- kept for good, docs/identity.md invariant 23), said for one new kind:
--
--   hunch -- a hunch said on the shop's screen; its `ref` is the phone's name
--            for the hunch (`<runtime>-<number>`), the one its lookup sends,
--            so a hunch no phrase did (`phrase_hunch`) finds its take by it.
--            Heard in English (`voice_repo::verdict::expected_language`) and
--            never told a phrase.
--
-- Nothing sits beside the note in this table's family, as for `interview`. A
-- new hunch is a new note under a new id; nothing is rewritten.
--
-- `voice_note` takes one more "said for". SQLite cannot alter a CHECK, so it
-- is rebuilt exactly as 20261113002100_add_interview_to_voice_note rebuilt
-- it: a fresh `voice_note_new` with the wider CHECK, every row copied across
-- unchanged, the old table dropped and the new one renamed in. Only the CHECK
-- on `of_kind` widens; every column, the append-only trigger, the
-- transcriber's index (`voice_note_by_stored_at`), the reconciler's index
-- (`voice_note_by_ref`) and the account trigger are recreated word for word.
-- The account trigger names `voice_note` in its body, so it is dropped first
-- and recreated once the rebuilt table holds the name again. Every other
-- table names a note by its id only and is untouched.
DROP TRIGGER voice_note_leaves_with_account;

CREATE TABLE voice_note_new (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene', 'game', 'studio', 'soundbite', 'lens', 'leetype', 'rust-round', 'interview', 'hunch')),
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

-- The reconciler reads one game's bites (20261113000500_create_bite_match).
CREATE INDEX voice_note_by_ref ON voice_note (of_kind, of_ref);

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER voice_note_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note WHERE subject_id = OLD.subject_id;
    DELETE FROM voice_note_audio WHERE subject_id = OLD.subject_id;
END;

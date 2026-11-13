-- An interview take is a voice note (paulgsc/some-ui#2932, IW1, epic
-- paulgsc/some-ui#2931, INTERVIEW-WEEK): each goes home like any other, kept
-- as `voice_note` and `voice_note_audio` keep every note (append-only, kept
-- for good, docs/identity.md invariant 23), said for one new kind:
--
--   interview -- a take said in a week of mock interview; its `ref` names
--                the question it answers, `<week>:<question>`, or the
--                teleprompter card it reads, `<week>:read:<card>`
--                (`voice_repo::is_interview_ref`).
--
-- Nothing sits beside the note in this table's family: no side table, as for
-- `rust-round`. A new take is a new note under a new id; nothing is
-- rewritten.
--
-- `voice_note` takes one more "said for". SQLite cannot alter a CHECK, so it
-- is rebuilt exactly as 20261102000100_add_rust_round_to_voice_note rebuilt
-- it: a fresh `voice_note_new` with the wider CHECK, every row copied across
-- unchanged, the old table dropped and the new one renamed in. Only the CHECK
-- on `of_kind` widens; every column, the append-only trigger, the
-- transcriber's index (`voice_note_by_stored_at`), the reconciler's index
-- (`voice_note_by_ref`, 20261113000500_create_bite_match) and the account
-- trigger are recreated word for word. The account trigger names
-- `voice_note` in its body, so it is dropped first and recreated once the
-- rebuilt table holds the name again. Every other table names a note by its
-- id only and is untouched.
DROP TRIGGER voice_note_leaves_with_account;

CREATE TABLE voice_note_new (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene', 'game', 'studio', 'soundbite', 'lens', 'leetype', 'rust-round', 'interview')),
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

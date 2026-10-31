-- Probe notes are voice notes (paulgsc/some-ui#2383, CK4, part of the CHECKS
-- epic, paulgsc/some-ui#2369): home checks its own transcription end to end by
-- putting a known clip through the real voice-note path, daily and when the
-- owner asks from the web page. The clip is a test fixture kept in this
-- repository (`file_host`'s `transcribe::probe`), synthesized speech of a
-- fixed line, never a person's recording. It is kept as `voice_note` and
-- `voice_note_audio` keep every note, said for `probe`, its `ref` the clip's
-- name, and heard by the transcriber like every note, so a probe exercises the
-- pass and the engine both. Whether it was heard right is `probe_run`'s and
-- `probe_result`'s (20261101000800).
--
-- A probe note is no person's: it is kept under the subject `probe`, which no
-- account holds or can hold (an account's subject is `subject-` and 32 hex
-- digits, `subject::new_account_subject`), so every reader that takes a
-- subject (the mailbox's, the routines', any count of a subject's notes) never
-- meets one, and account deletion never takes one. The phone cannot send one:
-- `voice_repo::describe` refuses `of: probe`.
--
-- `voice_note` takes a sixth "said for", `probe`. SQLite cannot alter a CHECK,
-- so it is rebuilt exactly as 20261030000100_add_studio_to_voice_note rebuilt
-- it for `studio`: a fresh `voice_note_new` with the wider CHECK, every row
-- copied across unchanged, the old table dropped and the new one renamed in.
-- Only the CHECK on `of_kind` widens; every column, the append-only trigger,
-- the transcriber's index (`voice_note_by_stored_at`) and the account trigger
-- are recreated word for word. The account trigger names `voice_note` in its
-- body, so it is dropped first and recreated once the rebuilt table holds the
-- name again. `voice_note_audio`, `game_bite`, `voice_transcript`,
-- `voice_transcript_word` and `mailbox_transcript` name a note by its id only
-- and are untouched.
DROP TRIGGER voice_note_leaves_with_account;

CREATE TABLE voice_note_new (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene', 'game', 'studio', 'probe')),
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

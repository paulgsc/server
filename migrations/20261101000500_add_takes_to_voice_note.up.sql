-- Soundbites, grammar-lens takes and LeetType's spoken answers are voice
-- notes (paulgsc/some-ui#2180, HF5, part of HOME-FIRST paulgsc/some-ui#2174;
-- this repository's issue for HF5). Until now each lived only on the phone:
-- a seventh soundbite pushed out the oldest, and lens takes and LeetType
-- takes stopped at forty, and the study-context export, a copy for the
-- owner's own agent, was the only way any of them left the phone. The owner
-- decided (paulgsc/some-ui#2174, D1) that home is the record: the phone
-- writes locally first, drains to home, and lets go only what home has read
-- back. Each recording is a note like any other, so it is kept as
-- `voice_note` and `voice_note_audio` keep every note (append-only, kept for
-- good, docs/identity.md invariant 23), said for one of three new kinds:
--
--   soundbite -- something the owner said on the soundbites page; `ref` is
--                how it came to be recorded (`direct`, `sessions`,
--                `reminder`, and the retired `capture` and `wrap`). What the
--                phone noted beside it (the sessions it knew of, its time
--                zone) is the soundbite's record in `study_note`, keyed by
--                the same id.
--   lens      -- the owner reading a grammar-lens line aloud after its
--                reveal; `ref` is the line, as the phone names it.
--   leetype   -- a spoken answer in a LeetType phone session; `ref` is the
--                session file and its screen (`<file>:<screen>`). What it
--                answered and the points ticked are the answer's note in
--                `study_note`, which names the take by this id.
--
-- Each `ref` meets the rule every `ref` meets, and nothing sits beside the
-- note in this table's family: no side table, as for `studio`. A new take is
-- a new note under a new id; nothing is ever rewritten.
--
-- `voice_note` takes three more "said for"s. SQLite cannot alter a CHECK,
-- so it is rebuilt exactly as 20261030000100_add_studio_to_voice_note
-- rebuilt it for `studio`: a fresh `voice_note_new` with the wider CHECK,
-- every row copied across unchanged, the old table dropped and the new one
-- renamed in. Only the CHECK on `of_kind` widens; every column, the
-- append-only trigger, the transcriber's index (`voice_note_by_stored_at`)
-- and the account trigger are recreated word for word. The account trigger
-- names `voice_note` in its body, so it is dropped first and recreated once
-- the rebuilt table holds the name again. `voice_note_audio`, `game_bite`,
-- `voice_transcript`, `voice_transcript_word` and `mailbox_transcript` name
-- a note by its id only and are untouched.
DROP TRIGGER voice_note_leaves_with_account;

CREATE TABLE voice_note_new (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene', 'game', 'studio', 'soundbite', 'lens', 'leetype')),
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

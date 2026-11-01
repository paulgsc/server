-- The phone's study notes (paulgsc/some-ui#2180, HF5, part of HOME-FIRST
-- paulgsc/some-ui#2174): the text that goes with a soundbite or a LeetType
-- spoken answer, kept on home as a record. Until now it lived only on the
-- phone, and the only way it left was the study-context export, a copy for
-- the owner's own agent; LeetType's answer notes were "Drive's, never home"
-- (paulgsc/some-ui#1934). The owner decided (paulgsc/some-ui#2174, D1) that
-- home is the record and the phone lets go only what home has read back.
--
-- A note is one of a closed list of kinds (`study_note_repo::Kind`):
--
--   soundbite -- a soundbite's record as the phone keeps it: when it was
--                recorded, how long it ran, its container and size, and what
--                the phone noted around it (how it came to be recorded, the
--                most recent session it knew of, how many were open, its
--                time zone). Keyed by the soundbite's id; its audio is the
--                voice note of that id (`voice_note`, said for `soundbite`).
--   leetype   -- a LeetType spoken answer's note: the session file and
--                screen, the one-sentence prompt, the rubric shown after
--                answering, the points the owner ticked (their own reading,
--                never a grade), the answer typed where they could not speak,
--                and the take it notes, by the voice note's id (`voice_note`,
--                said for `leetype`), or none.
--
-- It is a record, not a convenience copy, so it is APPEND-ONLY, as
-- `jobs_event` is. A note, once stored, is never updated (the trigger below
-- aborts an UPDATE) or replaced: `study_note_repo` refuses other bytes under
-- an id already held, and stores nothing for the same bytes again. The only
-- deletes are account deletion's.
--
-- Content only, never parsed, as `korean_entry` is: home checks that a body
-- is one JSON object within `study_note_repo::BODY_CEILING` and stores its
-- bytes verbatim but for the JSON whitespace around the value. What a note
-- says stays the phone's to check.
--
-- Columns of `study_note`:
--
--   subject_id -- whose record.
--   kind       -- `soundbite` or `leetype`, as above.
--   note_id    -- the phone's id for it (a UUID today), which is also what
--                 makes a retried send store nothing new.
--   body       -- the note's JSON as the phone sent it.
--   sha256     -- lowercase hex SHA-256 over `body`'s bytes: how a write of
--                 the bytes already held is told apart, and what the phone
--                 compares with what it sent before it lets its copy go.
--   stored_at  -- ISO-8601 UTC, when it reached home.
CREATE TABLE study_note (
    subject_id TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('soundbite', 'leetype')),
    note_id    TEXT NOT NULL,
    body       TEXT NOT NULL,
    sha256     TEXT NOT NULL,
    stored_at  TEXT NOT NULL,
    PRIMARY KEY (subject_id, kind, note_id)
);

CREATE TRIGGER study_note_is_append_only
BEFORE UPDATE ON study_note
BEGIN
    SELECT RAISE(ABORT, 'study_note is append-only: a note is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER study_note_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM study_note WHERE subject_id = OLD.subject_id;
END;

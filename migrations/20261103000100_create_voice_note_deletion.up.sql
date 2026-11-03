-- A receipt for each voice note the owner deleted for good from My data
-- (paulgsc/some-ui#2477, MD3, under epic #2473). The owner's rule
-- (paulgsc/some-ui#2474, 2026-11-02): the tool can go, never the work. A
-- recording whose purpose is served elsewhere may be deleted, kind by kind,
-- once the owner switches that kind on (`voice_repo::delete::SWITCHED_ON`,
-- empty until they do); what doing something produced never is. Every delete
-- leaves one of these, and a receipt is never deleted.
--
-- The earlier voice migrations say a note's only deletes are account
-- deletion's; this one adds the second, My data's, and those comments cannot
-- change (their checksums are recorded), so this is where that is amended.
--
-- Written only by `DELETE /api/v1/voice-notes/:id`
-- (`voice_repo::delete::delete`), in the transaction that deletes the note,
-- its audio and what home heard in it. It says what went and what stayed,
-- never what was said: no words, no ref (a take's ref names its project), no
-- audio and no hash.
--
-- It is a record, so it is APPEND-ONLY: a receipt is never updated (the
-- trigger below aborts an UPDATE), and goes only with the account, as every
-- subject-scoped table does (docs/identity.md invariant 22).
--
-- Columns of `voice_note_deletion`:
--
--   subject_id  -- whose note it was.
--   note_id     -- the note's id, the phone's (`voice_note.note_id`).
--   of_kind     -- what it was said for, as `voice_note.of_kind`.
--   recorded_at -- when the phone recorded it, as the note had it.
--   size        -- how many bytes its audio was.
--   stored_at   -- when it reached home, as the note had it.
--   deleted_at  -- ISO-8601 UTC, when home deleted it.
--   went        -- a JSON array of what went, in fixed words: `recording`,
--                  `words` (what home heard in it), `set-aside mark`.
--   stayed      -- a JSON array of what stayed, in fixed words: `mailbox
--                  copy` (its words filed for the Jobs mailbox),
--                  `drive copy` (the build-journal session listed it in
--                  Drive).
CREATE TABLE voice_note_deletion (
    subject_id  TEXT    NOT NULL,
    note_id     TEXT    NOT NULL,
    of_kind     TEXT    NOT NULL,
    recorded_at TEXT    NOT NULL,
    size        INTEGER NOT NULL CHECK (size > 0),
    stored_at   TEXT    NOT NULL,
    deleted_at  TEXT    NOT NULL,
    went        TEXT    NOT NULL CHECK (json_valid(went) AND json_type(went) = 'array'),
    stayed      TEXT    NOT NULL CHECK (json_valid(stayed) AND json_type(stayed) = 'array'),
    PRIMARY KEY (subject_id, note_id)
);

CREATE TRIGGER voice_note_deletion_is_append_only
BEFORE UPDATE ON voice_note_deletion
BEGIN
    SELECT RAISE(ABORT, 'voice_note_deletion is append-only: a receipt is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note` does.
CREATE TRIGGER voice_note_deletion_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note_deletion WHERE subject_id = OLD.subject_id;
END;

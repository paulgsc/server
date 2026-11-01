-- "Let go" on a voice take sets it aside and keeps it (the owner's D3 on
-- paulgsc/some-ui epic #2368, MIC2 paulgsc/some-ui#2372): the take leaves
-- its item and what the routines read, and is never deleted. The phone
-- already does this locally; this is home's mark for a note it holds.
--
-- It is its own table, not a column on `voice_note`, because a note is
-- never updated (docs/identity.md invariant 23: `voice_note`'s trigger
-- aborts every UPDATE), and a mark made after the note arrived would be
-- one. The same reason gave a said-first bite's later scenario its own
-- table (20261028000300_let_a_said_first_bite_keep_its_scenario).
--
-- It is a record, so it is APPEND-ONLY: a note is set aside once, by the
-- first mark, and never updated (the trigger aborts an UPDATE). A repeat
-- mark changes nothing, so the first mark's time stands. There is no
-- un-set: the phone has no undo for Let go. Setting a note aside deletes
-- nothing: the note, its audio and any transcript are kept as before. The
-- only deletes are account deletion's.
--
-- Written only by `PUT /api/v1/voice-notes/:id/set-aside`
-- (`voice_repo::set_aside::set_aside`), and only for a note the subject
-- holds. Read by what skips a set-aside note: the transcriber's queue
-- (`voice_repo::transcript::waiting`), the transcripts filed for the
-- mailbox (`mailbox_repo::unfiled_transcripts`) and the reflections index
-- (`mailbox_repo::reflections_inputs` and `reflections_as_of`). The Jobs
-- index is left as it is: the notes it waits on are the ones a decisions or
-- scouts file names, which a mark does not unname.
--
-- Nothing here is audio or anything said: the note is home's own id for a
-- note it holds, and the two times say when.
--
-- Columns of `voice_note_set_aside`:
--
--   subject_id   -- whose record.
--   note_id      -- the voice note (`voice_note.note_id`), the phone's id.
--   set_aside_at -- ISO-8601 UTC to the millisecond, when the owner let it
--                   go on the phone (the request's `at`).
--   stored_at    -- ISO-8601 UTC, when home recorded the mark.
CREATE TABLE voice_note_set_aside (
    subject_id   TEXT NOT NULL,
    note_id      TEXT NOT NULL,
    set_aside_at TEXT NOT NULL,
    stored_at    TEXT NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

CREATE TRIGGER voice_note_set_aside_is_append_only
BEFORE UPDATE ON voice_note_set_aside
BEGIN
    SELECT RAISE(ABORT, 'voice_note_set_aside is append-only: the first mark stands');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note` does.
CREATE TRIGGER voice_note_set_aside_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note_set_aside WHERE subject_id = OLD.subject_id;
END;

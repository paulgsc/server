-- Drama prompts (paulgsc/some-ui#2179, HF4): each next scene's prompt the
-- owner asks for on their phone ("Ask for the next scene", paulgsc/some-ui
-- `apps/www/src/lib/scene-ask`), sent home by the phone's drain. Home is the
-- owner's own store (paulgsc/some-ui#2174, decision D1), so it is the record:
-- the phone keeps one only until home holds it.
--
-- It is a record, not a convenience copy, so it is APPEND-ONLY. A prompt,
-- once stored, is never updated (the trigger below aborts an UPDATE) or
-- replaced: `drama_prompt_repo` refuses a second, different prompt under a
-- stamp it holds. The only deletes are account deletion's.
--
-- The prompt is the markdown the phone wrote (paulgsc/some-ui topik's
-- `nextSceneFile`): a note for an agent, a line naming each scene note by
-- the phone's id, and the tree prompt with its request and the last drama.
-- Its scene notes are not here: each went home as a voice note
-- (`voice_note`, docs/identity.md invariant 23). What the reflections routine
-- reads of it (its stamp and text) is filed for the Jobs mailbox's `prompt`
-- folder as it arrives (`mailbox_outgoing`, docs/identity.md invariant 19);
-- nothing else of it leaves home. Archived as a written reflection is, the
-- mailbox's answer to each ask to archive one is recorded in
-- `mailbox_archive_answer` (folder `prompt`), so one whose ask heard none is
-- asked for again.
--
-- Columns of `drama_prompt`:
--
--   subject_id -- whose record.
--   stamp      -- the phone's name for the prompt, `drama-` and the moment it
--                 was asked for in UTC to the millisecond
--                 (`drama-20261031T071503120Z`); the drama that answers it
--                 carries it too.
--   text       -- the prompt's markdown, as the phone wrote it. It holds the
--                 owner's own words (the scene asked for, the last drama's
--                 review), so it never reaches a log line.
--   written_at -- ISO-8601 UTC, when it reached home.
CREATE TABLE drama_prompt (
    subject_id TEXT NOT NULL,
    stamp      TEXT NOT NULL,
    text       TEXT NOT NULL,
    written_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, stamp)
);

CREATE TRIGGER drama_prompt_is_append_only
BEFORE UPDATE ON drama_prompt
BEGIN
    SELECT RAISE(ABORT, 'drama_prompt is append-only: ask for a new prompt');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `written_reflection` does.
CREATE TRIGGER drama_prompt_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM drama_prompt WHERE subject_id = OLD.subject_id;
END;

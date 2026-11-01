-- Home's word that Drive holds a Studio take too (paulgsc/some-ui#2401,
-- HF11, part of HOME-FIRST paulgsc/some-ui#2174). The phone keeps a take
-- home holds until the owner's choice lets it go; under "keep each until
-- Drive has it too", home confirming that Drive has it lets it go, so the
-- owner need not tap "Drive has it" on each.
--
-- Home gets no Drive credential (the owner's route a, 2026-10-31,
-- https://github.com/paulgsc/some-ui/issues/2401#issuecomment-6549482554).
-- The build-journal session, which already reads Drive's Build Journal takes
-- folder, puts a list of the SHA-256 of each recording it found there in the
-- Jobs mailbox (`brief`, `drive-takes-<stamp>.json`, schema
-- `some-ui.drive-takes/1`: SHA-256s and when it looked, nothing else). The
-- poller keeps the list as it keeps every `brief` file (`mailbox_item`,
-- docs/identity.md invariant 19) and, in the transaction that keeps it,
-- records here each `studio` voice note home holds whose SHA-256 the list
-- names (`voice_note.sha256`, ST25: the bytes the phone sent home are the
-- bytes it shared to Drive).
--
-- It is a record, so it is APPEND-ONLY: a take is recorded once, by the
-- first list that named it, and never updated (the trigger aborts an
-- UPDATE). A later list naming it again changes nothing; a later list that
-- no longer names it (the Drive SOP clears the takes folder after each
-- short) takes nothing away: Drive held it, and home holds it for good
-- (invariant 23). The only deletes are account deletion's.
--
-- Nothing here is audio, a Drive name, id or link, or anything the list did
-- not say: the note is home's own id for a take it holds, and the list's
-- name and time say which word of the session's it was.
--
-- Columns of `studio_take_on_drive`:
--
--   subject_id -- whose record.
--   note_id    -- the `studio` voice note (`voice_note.note_id`), the
--                 phone's id for the take.
--   listed_at  -- ISO-8601, when the session listed Drive's takes folder
--                 (the list's `at`).
--   list_name  -- the mailbox name of the list that first named it.
--   kept_at    -- ISO-8601 UTC, when home recorded it.
CREATE TABLE studio_take_on_drive (
    subject_id TEXT NOT NULL,
    note_id    TEXT NOT NULL,
    listed_at  TEXT NOT NULL,
    list_name  TEXT NOT NULL,
    kept_at    TEXT NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

CREATE TRIGGER studio_take_on_drive_is_append_only
BEFORE UPDATE ON studio_take_on_drive
BEGIN
    SELECT RAISE(ABORT, 'studio_take_on_drive is append-only: a take Drive held stays recorded');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note` does.
CREATE TRIGGER studio_take_on_drive_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM studio_take_on_drive WHERE subject_id = OLD.subject_id;
END;

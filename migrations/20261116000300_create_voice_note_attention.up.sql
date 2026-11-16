-- The owner's reconciliation of each take home could not hear
-- (paulgsc/server#912, HEARD3, epic paulgsc/some-ui#3070): one row per act
-- the owner took on a note whose newest verdict (`voice_transcript_verdict`,
-- HEARD1) is `garbled`, `failed` or `nothing_said`. The owner's words
-- (2026-11-16, on #912): "that page will be like a reconciliation page ...
-- and then as part of the reconciliation, I'll be choosing to delete, or
-- not". docs/identity.md invariant 23.
--
-- APPEND-ONLY. An act is never edited: a later act is a new row, numbered
-- after the note's last (`n`), and names the verdict it answered by that
-- verdict's key (model, attempt, rule). No row is updated (the trigger
-- below aborts an `UPDATE`) or deleted but with the account. It outlives
-- its note, as the verdicts do: deleting a note for good
-- (`voice_repo::delete`) writes a `deleted` act here when the note's newest
-- verdict was not `heard`, in the same transaction, and its receipt says
-- `reconciliation` stayed.
--
-- A note needs attention while its newest verdict (`voice_repo::verdict::
-- newest`) is not `heard` and no act here answers that verdict; a new
-- verdict (a better model hearing it again) is unanswered until the owner
-- acts on it.
--
-- Written only by `POST /api/v1/voice-notes/:id/attention` (`kept`,
-- `record_again`) and `DELETE /api/v1/voice-notes/:id` (`deleted`), both
-- `voice_repo::attention::act`. Read by `GET /api/v1/voice-notes/attention`.
--
-- Nothing here is audio or anything said: the note's id, the act, the
-- verdict's key and when.
--
-- Columns:
--
--   subject_id, note_id -- whose take.
--   n           -- the act's number on the note, from 1.
--   act         -- `kept`, `record_again` or `deleted`.
--   model, attempt, rule -- the verdict it answered.
--   acted_at    -- ISO-8601 UTC, when home recorded the act.
--
-- What it costs: one short row per act the owner takes.
CREATE TABLE voice_note_attention (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    act        TEXT    NOT NULL CHECK (act IN ('kept', 'record_again', 'deleted')),
    model      TEXT    NOT NULL,
    attempt    INTEGER NOT NULL CHECK (attempt >= 1),
    rule       TEXT    NOT NULL,
    acted_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id, n)
);

CREATE TRIGGER voice_note_attention_is_append_only
BEFORE UPDATE ON voice_note_attention
BEGIN
    SELECT RAISE(ABORT, 'voice_note_attention is append-only: a later act is a new row');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_transcript_verdict` does.
CREATE TRIGGER voice_note_attention_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note_attention WHERE subject_id = OLD.subject_id;
END;

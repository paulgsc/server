-- Transcripts filed for the Jobs mailbox (paulgsc/server#463, VOICE4): which
-- of home's transcripts (`voice_transcript`, 20261025000200) the `mailbox`
-- poller filed in `mailbox_outgoing`, and under what number. The routines
-- read the text there, in the mailbox's `transcript` folder (paulgsc/some-ui
-- `apps/mailbox`), instead of installing Whisper. docs/identity.md
-- invariant 19.
--
-- A mailbox name holds one body for good, and a better model's transcript of
-- a note must go under a name of its own, so the name carries a number:
-- `transcript-<note>-<n>.json`. `n` is decided once, here, as the note's
-- next, and never derived again: `voice_transcript`'s rowid is not
-- (a `VACUUM` may renumber it), and its `attempt` starts again at 1 for each
-- model. A row is one transcript, keyed by its note and model, since a model
-- ends a note at most once (`voice_transcript_one_ending`); only `heard` rows
-- are filed, so text, never a failure or a silence.
--
-- APPEND-ONLY, like the outgoing file it names: no row is updated (the
-- trigger below aborts an `UPDATE`) or deleted but with the account.
--
-- Columns:
--
--   subject_id -- whose transcript.
--   note_id    -- which note: `voice_note`'s id.
--   model      -- which model heard it, as `voice_transcript.model`.
--   n          -- its number among the note's filed transcripts, from 1.
--   filed_at   -- ISO-8601 UTC, when it was filed.
CREATE TABLE mailbox_transcript (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    model      TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    filed_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id, model),
    UNIQUE (subject_id, note_id, n)
);

CREATE TRIGGER mailbox_transcript_is_append_only
BEFORE UPDATE ON mailbox_transcript
BEGIN
    SELECT RAISE(ABORT, 'mailbox_transcript is append-only: a filed transcript keeps its number');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `mailbox_outgoing` does.
CREATE TRIGGER mailbox_transcript_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_transcript WHERE subject_id = OLD.subject_id;
END;

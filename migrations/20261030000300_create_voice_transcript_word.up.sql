-- When each heard word was said (paulgsc/some-ui#2085, GWB11): whisper.cpp's
-- per-word times beside each heard transcript (`voice_transcript`,
-- 20261025000200), so captions burned into a short can follow the voice word
-- by word. Approved by the owner (paulgsc/some-ui#2074, "Decided" 5 and 8).
-- docs/identity.md invariant 23.
--
-- APPEND-ONLY, like the transcripts. The words of a transcript are written
-- with it, in the same transaction, by `file_host`'s `transcribe` worker, and
-- only for a transcript that heard something; a transcript heard before this
-- table existed has none, and is never given any later. No row is updated
-- (the trigger below aborts an `UPDATE`) or deleted but with the account.
--
-- A transcript is named by its note and model: a model ends a note at most
-- once (`voice_transcript_one_ending`), so (subject_id, note_id, model) names
-- exactly one heard row, and its words are numbered from 0 in the order said.
--
-- Columns:
--
--   subject_id -- whose note.
--   note_id    -- which note: `voice_note`'s id.
--   model      -- the model that heard it, as `voice_transcript.model`.
--   word_index -- the word's place in what was heard, from 0.
--   text       -- the word as heard, punctuation that clings to it included
--                 (`said,`). Never logged.
--   start_ms   -- when the word began, in ms from the note's start (not from
--                 the speech the VAD cut out of it).
--   end_ms     -- when it ended, the same way.
--
-- What it costs: one row per word, about 150 a minute of speech (two and a
-- half words a second), each a few dozen bytes.
CREATE TABLE voice_transcript_word (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    model      TEXT    NOT NULL,
    word_index INTEGER NOT NULL CHECK (word_index >= 0),
    text       TEXT    NOT NULL,
    start_ms   INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms     INTEGER NOT NULL,
    PRIMARY KEY (subject_id, note_id, model, word_index),
    CHECK (end_ms >= start_ms)
);

CREATE TRIGGER voice_transcript_word_is_append_only
BEFORE UPDATE ON voice_transcript_word
BEGIN
    SELECT RAISE(ABORT, 'voice_transcript_word is append-only: a word heard is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_transcript` does.
CREATE TRIGGER voice_transcript_word_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_transcript_word WHERE subject_id = OLD.subject_id;
END;

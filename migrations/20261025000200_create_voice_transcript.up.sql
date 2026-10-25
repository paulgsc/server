-- Voice transcripts (paulgsc/server#462): what home heard in each voice note
-- (`voice_note`, 20261025000100), transcribed on home's CPU by `file_host`'s
-- `transcribe` worker with whisper.cpp. docs/identity.md invariant 23.
--
-- APPEND-ONLY, like the notes. Each row is one attempt to hear one note with
-- one model: what was heard, that nothing was said, or why it could not be
-- heard. A better model later adds rows of its own and never replaces one;
-- a reader takes the newest row that heard something. No row is updated (the
-- trigger below aborts an `UPDATE`) or deleted but with the account.
--
-- At most one row per note and model ends the matter (`heard` or `silent`,
-- held by the partial unique index below). A failure is kept too, so a note
-- whose audio cannot be heard is retried a few times (`transcribe::pass`'s
-- `ATTEMPTS`), never forever: the worker stops asking once a model has failed
-- on a note that many times.
--
-- Columns:
--
--   subject_id  -- whose note.
--   note_id     -- which note: `voice_note`'s id.
--   model       -- the model file it was heard with (`ggml-small-q5_1.bin`).
--   engine      -- what ran it: `whisper.cpp` and its version.
--   attempt     -- 1 for the first attempt with this model on this note, and
--                  one more for each after.
--   outcome     -- `heard` (text below), `silent` (nothing said: no speech
--                  found), or why it was not heard: `unreadable` (the audio
--                  did not decode), `too_long` (longer than the worker hears),
--                  `deadline` (it ran past its deadline) or `failed` (the
--                  engine failed).
--   language    -- for `heard`: the language detected, as Whisper's code
--                  (`en`, `ko`).
--   language_p  -- for `heard`: that detection's probability, 0 to 1.
--   duration_ms -- the audio's length, when it decoded.
--   text        -- for `heard`: what was heard. Never logged.
--   took_ms     -- how long the attempt took, decoding included.
--   heard_at    -- ISO-8601 UTC, when the attempt ended.
CREATE TABLE voice_transcript (
    subject_id  TEXT    NOT NULL,
    note_id     TEXT    NOT NULL,
    model       TEXT    NOT NULL,
    engine      TEXT    NOT NULL,
    attempt     INTEGER NOT NULL CHECK (attempt >= 1),
    outcome     TEXT    NOT NULL CHECK (outcome IN ('heard', 'silent', 'unreadable', 'too_long', 'deadline', 'failed')),
    language    TEXT,
    language_p  REAL,
    duration_ms INTEGER CHECK (duration_ms >= 0),
    text        TEXT,
    took_ms     INTEGER NOT NULL CHECK (took_ms >= 0),
    heard_at    TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id, model, attempt),
    -- Text and a language exactly when something was heard.
    CHECK ((outcome = 'heard') = (text IS NOT NULL)),
    CHECK ((outcome = 'heard') = (language IS NOT NULL AND language_p IS NOT NULL)),
    CHECK (outcome NOT IN ('heard', 'silent') OR duration_ms IS NOT NULL)
);

-- One ending per note and model: once heard (or found silent), a model never
-- hears that note again.
CREATE UNIQUE INDEX voice_transcript_one_ending
    ON voice_transcript (subject_id, note_id, model)
    WHERE outcome IN ('heard', 'silent');

-- The worker takes the oldest notes first.
CREATE INDEX voice_note_by_stored_at ON voice_note (stored_at, subject_id, note_id);

CREATE TRIGGER voice_transcript_is_append_only
BEFORE UPDATE ON voice_transcript
BEGIN
    SELECT RAISE(ABORT, 'voice_transcript is append-only: a transcript is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note` does.
CREATE TRIGGER voice_transcript_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_transcript WHERE subject_id = OLD.subject_id;
END;

-- What home made of each transcript it keeps (paulgsc/server#908, HEARD1,
-- epic paulgsc/some-ui#3070): heard, garbled, nothing said, or failed,
-- judged by `file_host`'s `transcribe` worker from the transcriber's own
-- signals, so no reader of a transcript judges it for itself and a garbled
-- take is found at home, not in a routine. docs/identity.md invariant 23.
--
-- APPEND-ONLY, like the transcripts. A verdict is written with its attempt
-- (`voice_transcript`, 20261025000200), in the same transaction, and names
-- it by (subject_id, note_id, model, attempt). A later rule judging the same
-- attempt again adds a row under its own `rule`; no row is updated (the
-- trigger below aborts an `UPDATE`) or deleted but with the account. It
-- outlives its note: when the owner deletes a note for good
-- (`voice_repo::delete`), its transcripts go and its verdicts stay, holding
-- no word, only codes, numbers and a language, so a take's problems are
-- kept (the owner's call, paulgsc/server#911); its receipt says `verdict`
-- stayed. A transcript kept before this table existed has none, and reads
-- say so.
--
-- Only final attempts are judged: one that ended the note (`heard`, `silent`)
-- and the last failure the worker makes on it (`transcribe::pass::ATTEMPTS`).
-- A failure that will be retried says nothing final, so it has no verdict.
--
-- Columns:
--
--   subject_id, note_id, model, attempt -- the attempt judged.
--   rule        -- which thresholds judged it (`heard-1`).
--   verdict     -- `heard`, `garbled`, `nothing_said` or `failed`.
--   reasons     -- why, space-separated: for `garbled` any of
--                  `low_confidence repetitive no_speech off_language sparse`,
--                  for `failed` one of `unreadable too_long deadline engine`;
--                  empty otherwise.
--   avg_logprob .. rms_dbfs -- the signals it was judged on, for `heard` and
--                  `garbled` only (`voice_repo::verdict::Signals` says each).
--   judged_at   -- ISO-8601 UTC, when it was judged.
--
-- What it costs: one row of a few dozen bytes per note heard.
CREATE TABLE voice_transcript_verdict (
    subject_id        TEXT    NOT NULL,
    note_id           TEXT    NOT NULL,
    model             TEXT    NOT NULL,
    attempt           INTEGER NOT NULL CHECK (attempt >= 1),
    rule              TEXT    NOT NULL,
    verdict           TEXT    NOT NULL CHECK (verdict IN ('heard', 'garbled', 'nothing_said', 'failed')),
    reasons           TEXT    NOT NULL,
    avg_logprob       REAL,
    compression_ratio REAL,
    no_speech_prob    REAL,
    unsure_share      REAL,
    language          TEXT,
    language_p        REAL,
    expected_language TEXT,
    words_per_second  REAL,
    speech_ms         INTEGER CHECK (speech_ms >= 0),
    peak_dbfs         REAL,
    rms_dbfs          REAL,
    judged_at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id, model, attempt, rule),
    -- Signals exactly when something was heard.
    CHECK ((verdict IN ('heard', 'garbled')) = (speech_ms IS NOT NULL))
);

CREATE TRIGGER voice_transcript_verdict_is_append_only
BEFORE UPDATE ON voice_transcript_verdict
BEGIN
    SELECT RAISE(ABORT, 'voice_transcript_verdict is append-only: a verdict is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_transcript` does.
CREATE TRIGGER voice_transcript_verdict_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_transcript_verdict WHERE subject_id = OLD.subject_id;
END;

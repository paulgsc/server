-- Home's probes (paulgsc/some-ui#2383, CK4, part of the CHECKS epic,
-- paulgsc/some-ui#2369): each time home checks one of its own chains end to
-- end with a known input, and what came of it. `file_host`'s
-- `transcribe::probe` is the one writer; `voice_repo::probe` is the one
-- module that writes these tables.
--
-- Not subject-scoped: a probe runs a test fixture (a known clip, synthesized
-- speech of a fixed line) through home's real path, so a run is about home,
-- never about a person. No column names one, and the page that shows a run
-- (paulgsc/some-ui#2388, CK8) is the operator's.
--
-- APPEND-ONLY, like the notes the transcription probe puts through. A run is
-- written when it starts and its result once, when it ends; the triggers
-- abort every `UPDATE` and `DELETE`, and the result's primary key refuses a
-- second ending. What a run reads back is a verdict, timings and the engine's
-- name, never the words heard: those are the probe note's transcript
-- (`voice_transcript`, under the subject `probe`).
--
-- Columns of `probe_run`:
--
--   run_id     -- `probe-` and 32 hex digits, random; also the probe note's
--                 id for the transcription probe.
--   probe      -- which probe: `transcription` (a known clip, put through
--                 the voice-note path and heard by the real engine).
--   asked_by   -- `schedule` (home's daily run) or `page` (the operator
--                 asked from the web page).
--   started_at -- ISO-8601 UTC, when the run started.
--
-- Columns of `probe_result` (at most one per run):
--
--   run_id     -- which run.
--   verdict    -- `pass` or `fail`.
--   outcome    -- how it ended: for the transcription probe, the
--                 transcript's outcome (`heard`, `silent`, `unreadable`,
--                 `too_long`, `deadline`, `failed`), or `timed_out` when no
--                 ending came within the probe's bound.
--   word_error -- for `heard`: the word error rate of what was heard against
--                 the clip's expected words, 0 and up.
--   waited_ms  -- from the run's start to its ending: the time to transcript.
--   took_ms    -- how long the engine took on the attempt that ended it.
--   model      -- the model file it was heard with.
--   engine     -- what ran it, and its version (`whisper.cpp 1.8.3`).
--   ended_at   -- ISO-8601 UTC, when the result was written.
CREATE TABLE probe_run (
    run_id     TEXT NOT NULL PRIMARY KEY,
    probe      TEXT NOT NULL CHECK (probe IN ('transcription')),
    asked_by   TEXT NOT NULL CHECK (asked_by IN ('schedule', 'page')),
    started_at TEXT NOT NULL
);

-- The newest run of a probe: what "is one due" and the page read.
CREATE INDEX probe_run_by_started_at ON probe_run (probe, started_at);

CREATE TABLE probe_result (
    run_id     TEXT    NOT NULL PRIMARY KEY,
    verdict    TEXT    NOT NULL CHECK (verdict IN ('pass', 'fail')),
    outcome    TEXT    NOT NULL CHECK (outcome IN ('heard', 'silent', 'unreadable', 'too_long', 'deadline', 'failed', 'timed_out')),
    word_error REAL    CHECK (word_error >= 0),
    waited_ms  INTEGER NOT NULL CHECK (waited_ms >= 0),
    took_ms    INTEGER CHECK (took_ms >= 0),
    model      TEXT,
    engine     TEXT,
    ended_at   TEXT    NOT NULL,
    -- A word error exactly when something was heard; a pass only then.
    CHECK ((outcome = 'heard') = (word_error IS NOT NULL)),
    CHECK (verdict = 'fail' OR outcome = 'heard')
);

CREATE TRIGGER probe_run_is_append_only
BEFORE UPDATE ON probe_run
BEGIN
    SELECT RAISE(ABORT, 'probe_run is append-only: a run is never rewritten');
END;

CREATE TRIGGER probe_run_is_never_deleted
BEFORE DELETE ON probe_run
BEGIN
    SELECT RAISE(ABORT, 'probe_run is append-only: a run is never deleted');
END;

CREATE TRIGGER probe_result_is_append_only
BEFORE UPDATE ON probe_result
BEGIN
    SELECT RAISE(ABORT, 'probe_result is append-only: a result is never rewritten');
END;

CREATE TRIGGER probe_result_is_never_deleted
BEFORE DELETE ON probe_result
BEGIN
    SELECT RAISE(ABORT, 'probe_result is append-only: a result is never deleted');
END;

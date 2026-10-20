-- The LeetType session log (paulgsc/some-ui
-- `packages/ui/leetype/src/lib/leetype/session-log`): every phone session of
-- incident rounds, as the append-only list of what happened in it. A session
-- started from a session file, each pick and how long it took, the readings
-- placed, opened, saved or read, the rounds parked, returned and confirmed,
-- each spoken answer and how it went, and how the session ended. Once the
-- phone sends a finished session home ("Send to home"), this table is its
-- record, and the phone lets go of what home holds.
--
-- It is a record, not a convenience copy, so it is APPEND-ONLY, as the
-- phone's log is. An event, once stored, is never updated (the trigger below
-- aborts an UPDATE) or replaced: `leetype_log_repo` refuses a second,
-- different body under an id the session holds. The only deletes are account
-- deletion's. A later event says what changed (a round parked, then returned
-- and solved, is solved), the way the phone reads its own log.
--
-- Soundbites never come here. A spoken answer's ticked rubric points, the
-- prompt, the rubric and anything typed go to the owner's Drive with the
-- phone's agent export; `leetype_log_repo` refuses an event that carries
-- them. What comes home of a spoken answer is how it was given, for how many
-- seconds, and whether it was strong, which is what the ledger reads.
--
-- Columns of `leetype_session_event`:
--
--   subject_id -- whose record.
--   session_id -- the phone's id for the session (a UUID today).
--   event_id   -- the phone's id for the event (a UUID today).
--   at         -- when the phone recorded it, as ISO-8601 UTC to the
--                 millisecond: the order a read serves. Also in `body`, as
--                 the phone wrote it.
--   type       -- the event's type (`started`, `picked`, ...), as in `body`.
--   body       -- the event's JSON as `leetype_log_repo` re-serialised it
--                 after checking it, which is what a read serves.
--   written_at -- ISO-8601 UTC, when it reached home.
CREATE TABLE leetype_session_event (
    subject_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    event_id   TEXT NOT NULL,
    at         TEXT NOT NULL,
    type       TEXT NOT NULL,
    body       TEXT NOT NULL,
    written_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, session_id, event_id)
);

-- A session's events are read by the primary key's prefix. The list of
-- sessions and any read across sessions go by when they were recorded: by
-- `at`, then by insertion (`rowid`, which every SQLite index carries), since
-- the phone sends a session's log in order and two events can share a
-- millisecond.
CREATE INDEX leetype_session_event_order ON leetype_session_event (subject_id, at);

CREATE TRIGGER leetype_session_event_is_append_only
BEFORE UPDATE ON leetype_session_event
BEGIN
    SELECT RAISE(ABORT, 'leetype_session_event is append-only: write a new event');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER leetype_session_event_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM leetype_session_event WHERE subject_id = OLD.subject_id;
END;

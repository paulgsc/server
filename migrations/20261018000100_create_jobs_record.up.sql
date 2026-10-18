-- Jobs (paulgsc/some-ui `apps/www/src/lib/jobs`): the owner's record of the
-- daily job picks the phone works through, and what came of each. On the
-- phone it is an append-only log of events in one `localStorage` key: a pick
-- taken or passed and why, the résumé built, the posting opened, how applying
-- went, outcomes later, postings found by hand, and the phone's own shares.
-- Once the phone keeps it on home, this table is its record.
--
-- It is a record, not a convenience copy, so it is APPEND-ONLY, as the
-- phone's log is. An event, once stored, is never updated (the trigger below
-- aborts an UPDATE) or replaced: `jobs_repo` refuses a second, different body
-- under an id it holds. The only deletes are account deletion's. A later
-- event says what changed (a pick passed, then taken, is taken), the way the
-- phone reads its own log.
--
-- Voice notes stay on the phone and in the owner's Drive: an event names its
-- notes by the phone's ids for them, and no audio is ever stored here.
--
-- Columns of `jobs_event`:
--
--   subject_id -- whose record.
--   event_id   -- the phone's id for the event (a UUID today).
--   at         -- when the phone recorded it, as ISO-8601 UTC to the
--                 millisecond: the order a read serves. Also in `body`, as
--                 the phone wrote it.
--   body       -- the event's JSON as `jobs_repo` re-serialised it after
--                 checking it, which is what a read serves.
--   written_at -- ISO-8601 UTC, when it reached home.
CREATE TABLE jobs_event (
    subject_id TEXT NOT NULL,
    event_id   TEXT NOT NULL,
    at         TEXT NOT NULL,
    body       TEXT NOT NULL,
    written_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, event_id)
);

-- A read serves a subject's events in the order they were recorded.
CREATE INDEX jobs_event_order ON jobs_event (subject_id, at, event_id);

CREATE TRIGGER jobs_event_is_append_only
BEFORE UPDATE ON jobs_event
BEGIN
    SELECT RAISE(ABORT, 'jobs_event is append-only: write a new event');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `aph_entry` does.
CREATE TRIGGER jobs_event_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM jobs_event WHERE subject_id = OLD.subject_id;
END;

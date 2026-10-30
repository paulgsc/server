-- Studio's time log and friction reports (paulgsc/some-ui ST9, #2257; this
-- repository's #553): what the phone's Studio logs of each sitting, kept on
-- home as a record, never on Drive.
--
-- A sitting is one stretch of work on a video of a kind (`record`, `watch`,
-- `shape` or `finish`). The phone's Studio logs each screen entered and left,
-- the page hidden and shown, and the sitting opened and closed; time per
-- screen is a fold over that log, which the phone does, not home. At the end
-- of a sitting the owner says how it went (`smooth`, `some_snags` or
-- `rough`), may name a cause, and may leave a voice note; that is a friction
-- report.
--
-- Both tables are records, not convenience copies, so both are APPEND-ONLY,
-- as `leetype_session_event` is. A row, once stored, is never updated (the
-- triggers below abort an UPDATE) or replaced: `studio_log_repo` refuses a
-- second, different body under an id already held. The only deletes are
-- account deletion's. A voice note is named by the phone's id only; its audio
-- is `voice_note_audio`'s.
--
-- Columns of `studio_sitting_event`:
--
--   subject_id -- whose record.
--   sitting_id -- the phone's id for the sitting (a UUID today).
--   event_id   -- the phone's id for the event (a UUID today), which is also
--                 what makes a retried send store nothing new.
--   at         -- when the phone recorded it, as ISO-8601 UTC to the
--                 millisecond: the order a read serves. Also in `body`.
--   type       -- what happened (`screen_entered`, `page_hidden`, ...), as
--                 in `body`.
--   kind       -- the sitting's kind, as in `body`; every event of a sitting
--                 carries the same one.
--   body       -- the event's JSON as `studio_log_repo` re-serialised it
--                 after checking it, which is what a read serves.
--   written_at -- ISO-8601 UTC, when it reached home.
CREATE TABLE studio_sitting_event (
    subject_id TEXT NOT NULL,
    sitting_id TEXT NOT NULL,
    event_id   TEXT NOT NULL,
    at         TEXT NOT NULL,
    type       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    body       TEXT NOT NULL,
    written_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, sitting_id, event_id)
);

-- A sitting's read-back, in the order recorded: by `at`, then by insertion
-- (`rowid`, which every SQLite index carries). The primary key serves a
-- put's lookup by id.
CREATE INDEX studio_sitting_event_order ON studio_sitting_event (subject_id, sitting_id, at);

CREATE TRIGGER studio_sitting_event_is_append_only
BEFORE UPDATE ON studio_sitting_event
BEGIN
    SELECT RAISE(ABORT, 'studio_sitting_event is append-only: write a new event');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER studio_sitting_event_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM studio_sitting_event WHERE subject_id = OLD.subject_id;
END;

-- Columns of `studio_friction`:
--
--   subject_id -- whose record.
--   report_id  -- the phone's id for the report (a UUID today), which is
--                 also what makes a retried send store nothing new.
--   sitting_id -- the sitting it reports on, as in `body`.
--   at         -- when the phone recorded it, as ISO-8601 UTC to the
--                 millisecond: the order a read serves. Also in `body`.
--   how        -- how it went (`smooth`, `some_snags`, `rough`), as in `body`.
--   body       -- the report's JSON as `studio_log_repo` re-serialised it
--                 after checking it: the sitting's kind, how it went, the
--                 cause named (at most 200 characters, possibly typed by the
--                 owner), the shot it was on and a voice note's id, the last
--                 three each optional.
--   written_at -- ISO-8601 UTC, when it reached home.
CREATE TABLE studio_friction (
    subject_id TEXT NOT NULL,
    report_id  TEXT NOT NULL,
    sitting_id TEXT NOT NULL,
    at         TEXT NOT NULL,
    how        TEXT NOT NULL,
    body       TEXT NOT NULL,
    written_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, report_id)
);

-- The reports read newest first: by `at`, then by insertion.
CREATE INDEX studio_friction_order ON studio_friction (subject_id, at);

CREATE TRIGGER studio_friction_is_append_only
BEFORE UPDATE ON studio_friction
BEGIN
    SELECT RAISE(ABORT, 'studio_friction is append-only: write a new report');
END;

CREATE TRIGGER studio_friction_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM studio_friction WHERE subject_id = OLD.subject_id;
END;

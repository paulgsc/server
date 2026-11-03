-- What the app saw happen as it ran (paulgsc/some-ui CK2, #2381, under the
-- CHECKS epic #2369): one row per obligation the app ran to an ending on the
-- phone (a take, and later a listening, a spoken line, a note sent home),
-- saying how it ended, how long it took, which screen it came from and
-- which build. It is how the owner learns a process stopped working without
-- walking the exact steps that reach the failure.
--
-- One table for every family of event, so later families ride the same
-- route and the same rows (the owner, 2026-11-03: ENGAGE's engagement
-- events, paulgsc/some-ui ENG2 #2535, are the second). A family is a value
-- of `family` and a shape of `body` that `app_event_repo` checks; a new one
-- is a new variant there, not a new table. There is no CHECK on `family`, so
-- a family is added without rebuilding the table.
--
-- What an event may carry is the owner's C3 (paulgsc/some-ui#2369): the
-- ending, timings and the screen, by the code's own names. No audio, no
-- words, no item content: every text field is an id (lowercase letters,
-- digits, `-` and `.`), so nothing typed or said has anywhere to go
-- (docs/identity.md invariant 34).
--
-- It is a record of what happened, so it is APPEND-ONLY: a row, once
-- stored, is never updated (the trigger aborts an UPDATE) or replaced
-- (`app_event_repo` refuses a second, different body under an id already
-- held). The only deletes are account deletion's.
--
-- Columns of `app_event`:
--
--   subject_id -- whose events.
--   event_id   -- the phone's id for the event (a UUID today), which is
--                 also what makes a retried send store nothing new.
--   family     -- which family of event (`outcome`), as in `body`.
--   at         -- when the event's obligation started on the phone, as
--                 ISO-8601 UTC to the millisecond: what a window reads by.
--                 Also in `body`.
--   body       -- the event's JSON as `app_event_repo` re-serialised it
--                 after checking it, which is what a write answers with.
--   written_at -- ISO-8601 UTC, when it reached home.
CREATE TABLE app_event (
    subject_id TEXT NOT NULL,
    event_id   TEXT NOT NULL,
    family     TEXT NOT NULL,
    at         TEXT NOT NULL,
    body       TEXT NOT NULL,
    written_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, event_id)
);

-- A summary reads one subject's family over a window: by `at`. The primary
-- key serves a put's lookup by id.
CREATE INDEX app_event_by_family ON app_event (subject_id, family, at);

CREATE TRIGGER app_event_is_append_only
BEFORE UPDATE ON app_event
BEGIN
    SELECT RAISE(ABORT, 'app_event is append-only: write a new event');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER app_event_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM app_event WHERE subject_id = OLD.subject_id;
END;

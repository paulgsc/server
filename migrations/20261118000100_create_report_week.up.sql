-- Each week's engagement report, as home first wrote it (paulgsc/some-ui
-- ENG12, #2700; paulgsc/server#821). Once a week has ended on the owner's
-- clock, file_host's `report_weeks` worker reads the owner's engagement for
-- it with `app_event_repo::weekly` (the body `GET /app-events/weeks/:week`
-- answers) and keeps that answer here, once: a week already held is never
-- read again, so events that reach home after it was written change nothing,
-- and the render worker is given exactly these bytes
-- (`report-weeks/<week>.json`). A video re-rendered from it is the same
-- video. `app_event_repo::report` is the one module that writes it.
--
-- SUBJECT-SCOPED: the body is the owner's raw counts per pipeline,
-- affordance and surface, with days and no time of day, as `app_event` holds
-- them; what may go public, and how coarsely, is the report writer's rule
-- in some-ui, applied when it is rendered. It leaves with the account, and
-- so does its file on the drive (`report_weeks`'s tidy).
--
--   subject_id -- whose.
--   week       -- the ISO week on the owner's clock, `2026-W46`.
--   body       -- the weekly read's JSON, exactly as written to the drive.
--   written_at -- ISO-8601 UTC, when home wrote it.
--
-- APPEND-ONLY: no row is updated, and none is deleted while the subject's
-- account stands.
CREATE TABLE report_week (
    subject_id TEXT NOT NULL,
    week       TEXT NOT NULL CHECK (length(week) = 8 AND week GLOB '[0-9][0-9][0-9][0-9]-W[0-9][0-9]'),
    body       TEXT NOT NULL,
    written_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week)
);

CREATE TRIGGER report_week_is_append_only
BEFORE UPDATE ON report_week
BEGIN
    SELECT RAISE(ABORT, 'report_week is append-only: a week is kept as first written');
END;

CREATE TRIGGER report_week_is_kept
BEFORE DELETE ON report_week
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'report_week is append-only: a week goes only with its account');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER report_week_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM report_week WHERE subject_id = OLD.subject_id;
END;

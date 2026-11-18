-- Each week's engagement report video, once it is on YouTube, and each step
-- of letting it go (paulgsc/some-ui ENG12, #2700; paulgsc/server#821), as
-- the call's and the essays' are (`call_video_*`, `rust_essay_video_*`):
-- home renders each week it keeps in `report_week` into
-- `renders/report/<week>/` on the database's drive, the owner watches it on
-- the phone, shares it to the YouTube app, records its link, and lets home's
-- copy go from My data, the video first, then the rest of its folder. The
-- week stays in `report_week`, so a week let go is never rendered again but
-- can always be read. `app_event_repo::report_video` is the one module that
-- writes these tables.
--
-- SUBJECT-SCOPED, as `report_week` is: the report is written from the
-- owner's own records. Both tables leave with the account.
--
-- `report_video_link`, a link per week and YouTube video, the newest
-- standing:
--
--   subject_id  -- whose.
--   week        -- the report's ISO week (`2026-W46`).
--   youtube_id  -- the video's id on YouTube, eleven characters.
--   recorded_at -- when the owner recorded it.
--
-- `report_video_deletion`, a receipt per step of a week let go:
--
--   subject_id  -- whose.
--   week        -- the report's week.
--   step        -- `video`, then `rest` (the rest of its folder).
--   youtube_id  -- its link when it was let go.
--   size        -- the bytes that went.
--   deleted_at  -- when.
--
-- A week with a `video` receipt is never given to the render worker again
-- (`report_weeks`'s export).
--
-- BOTH TABLES ARE APPEND-ONLY: no row is updated, and none is deleted while
-- the subject's account stands.
CREATE TABLE report_video_link (
    subject_id  TEXT NOT NULL,
    week        TEXT NOT NULL,
    youtube_id  TEXT NOT NULL CHECK (length(youtube_id) = 11 AND youtube_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week, youtube_id)
);

CREATE TABLE report_video_deletion (
    subject_id TEXT    NOT NULL,
    week       TEXT    NOT NULL,
    step       TEXT    NOT NULL CHECK (step IN ('video', 'rest')),
    youtube_id TEXT    NOT NULL,
    size       INTEGER NOT NULL CHECK (size >= 0),
    deleted_at TEXT    NOT NULL,
    PRIMARY KEY (subject_id, week, step)
);

CREATE TRIGGER report_video_deletion_rest_after_video
BEFORE INSERT ON report_video_deletion
WHEN NEW.step = 'rest'
 AND NOT EXISTS (
    SELECT 1 FROM report_video_deletion
    WHERE subject_id = NEW.subject_id AND week = NEW.week AND step = 'video'
 )
BEGIN
    SELECT RAISE(ABORT, 'report_video_deletion: the rest of a week goes only after its video');
END;

-- No foreign keys, as the essays' tables have none: account deletion's
-- triggers and sweep take these tables in no fixed order with
-- `report_week`. These two triggers hold what a key would.
CREATE TRIGGER report_video_link_names_a_week
BEFORE INSERT ON report_video_link
WHEN NOT EXISTS (SELECT 1 FROM report_week WHERE subject_id = NEW.subject_id AND week = NEW.week)
BEGIN
    SELECT RAISE(ABORT, 'report_video_link: a link is recorded only for a week home keeps');
END;

CREATE TRIGGER report_video_deletion_needs_its_link
BEFORE INSERT ON report_video_deletion
WHEN NOT EXISTS (
    SELECT 1 FROM report_video_link
    WHERE subject_id = NEW.subject_id AND week = NEW.week AND youtube_id = NEW.youtube_id
 )
BEGIN
    SELECT RAISE(ABORT, 'report_video_deletion: a week is let go only once its link is recorded');
END;

CREATE TRIGGER report_video_link_is_append_only
BEFORE UPDATE ON report_video_link
BEGIN
    SELECT RAISE(ABORT, 'report_video_link is append-only: a recorded link is never rewritten');
END;

CREATE TRIGGER report_video_link_is_kept
BEFORE DELETE ON report_video_link
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'report_video_link is append-only: a link goes only with its account');
END;

CREATE TRIGGER report_video_deletion_is_append_only
BEFORE UPDATE ON report_video_deletion
BEGIN
    SELECT RAISE(ABORT, 'report_video_deletion is append-only: a receipt is never rewritten');
END;

CREATE TRIGGER report_video_deletion_is_kept
BEFORE DELETE ON report_video_deletion
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'report_video_deletion is append-only: a receipt goes only with its account');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER report_video_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM report_video_deletion WHERE subject_id = OLD.subject_id;
    DELETE FROM report_video_link WHERE subject_id = OLD.subject_id;
END;

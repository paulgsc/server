-- The weekly call's videos, once they are on YouTube (STUDIO ST38:
-- paulgsc/some-ui#2661, the owner's decisions of 2026-11-07). Home renders
-- each week's video into `renders/call/<from>/` on the database's drive
-- (ST36). The owner watches it on the phone, shares it to the YouTube app,
-- records the week's YouTube link from the phone, and then lets home's copy
-- go from My data, in two steps: the video, then the rest of its folder
-- (takes, stills, cut list). The week's words stay in `call_week`, so a week
-- let go can always be rendered again. `call_week_repo::video` is the one
-- module that writes these tables.
--
-- Not subject-scoped, as `call_week` is not: the call says what merged in two
-- public repositories, and its video is public on YouTube. No column names a
-- person.
--
-- Both tables are APPEND-ONLY: the triggers abort every `UPDATE` and
-- `DELETE`.
--
-- `call_video_link`: each YouTube video the owner recorded for a week, once
-- per video; the newest recorded is the week's link. A link recorded by
-- mistake is answered by recording the right one, never by rewriting.
--
--   week_from   -- the week, as `call_week.week_from`.
--   youtube_id  -- the video's id, eleven characters of `[A-Za-z0-9_-]`;
--                  its link is `https://youtu.be/<youtube_id>`.
--   recorded_at -- ISO-8601 UTC, when home recorded it.
CREATE TABLE call_video_link (
    week_from   TEXT NOT NULL REFERENCES call_week (week_from),
    youtube_id  TEXT NOT NULL CHECK (length(youtube_id) = 11 AND youtube_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (week_from, youtube_id)
);

CREATE TRIGGER call_video_link_no_update
BEFORE UPDATE ON call_video_link
BEGIN
    SELECT RAISE(ABORT, 'call_video_link is append-only: a recorded link is never rewritten');
END;

CREATE TRIGGER call_video_link_no_delete
BEFORE DELETE ON call_video_link
BEGIN
    SELECT RAISE(ABORT, 'call_video_link is append-only: a recorded link is never deleted');
END;

-- `call_video_deletion`: a receipt for each step of a week the owner let go
-- from My data, at most one per week and step, kept for good. The `rest`
-- step needs the `video` step's receipt first (checked here, as well as by
-- the repository), and the `video` step a recorded link.
--
--   week_from  -- the week, as `call_week.week_from`.
--   step       -- `video` (its `call.mp4`) or `rest` (the rest of its folder).
--   youtube_id -- the week's link when it was let go.
--   size       -- how many bytes went.
--   deleted_at -- ISO-8601 UTC, when home deleted them.
CREATE TABLE call_video_deletion (
    week_from  TEXT    NOT NULL REFERENCES call_week (week_from),
    step       TEXT    NOT NULL CHECK (step IN ('video', 'rest')),
    youtube_id TEXT    NOT NULL,
    size       INTEGER NOT NULL CHECK (size >= 0),
    deleted_at TEXT    NOT NULL,
    PRIMARY KEY (week_from, step),
    FOREIGN KEY (week_from, youtube_id) REFERENCES call_video_link (week_from, youtube_id)
);

CREATE TRIGGER call_video_deletion_rest_after_video
BEFORE INSERT ON call_video_deletion
WHEN NEW.step = 'rest'
 AND NOT EXISTS (SELECT 1 FROM call_video_deletion WHERE week_from = NEW.week_from AND step = 'video')
BEGIN
    SELECT RAISE(ABORT, 'call_video_deletion: the rest of a week goes only after its video');
END;

CREATE TRIGGER call_video_deletion_no_update
BEFORE UPDATE ON call_video_deletion
BEGIN
    SELECT RAISE(ABORT, 'call_video_deletion is append-only: a receipt is never rewritten');
END;

CREATE TRIGGER call_video_deletion_no_delete
BEFORE DELETE ON call_video_deletion
BEGIN
    SELECT RAISE(ABORT, 'call_video_deletion is append-only: a receipt is never deleted');
END;

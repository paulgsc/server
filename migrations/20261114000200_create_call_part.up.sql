-- The weekly call's videos, a part at a time (paulgsc/some-ui#2918, the
-- owner's direction of 2026-11-14). From template version 5 a long week is a
-- series of parts, each a video of at most ten minutes, each put on YouTube
-- and let go on its own. So each part's link and receipts stand on their
-- own: they are keyed by (week, part), where part 1 of a week of one part is
-- exactly what `call_video_link` and `call_video_deletion` held for it.
-- `call_week_repo::video` is the one module that writes these tables.
--
-- Not subject-scoped, as `call_week` is not: the call says what merged in two
-- public repositories, and its videos are public on YouTube. No column names
-- a person.
--
-- **Nothing is reshaped in place, and nothing is dropped** (paulgsc/some-ui
-- CLAUDE.md, "Never lose the record"). The tables of 20261107000100 stay as they
-- are, every row in them, as the durable copy of what home recorded before
-- parts; their rows are copied here as part 1 (a week then was one video),
-- and from now on they take no new row: a new trigger aborts every INSERT,
-- so a build from before this migration, run against it by a rollback,
-- fails loudly where it would write, never into a table nothing reads.
--
-- Both new tables are APPEND-ONLY: the triggers abort every `UPDATE` and
-- `DELETE`.
--
-- `call_part_link`: each YouTube video the owner recorded for a part of a
-- week, once per video; the newest recorded is the part's link. A link
-- recorded by mistake is answered by recording the right one.
--
--   week_from   -- the week, as `call_week.week_from`.
--   part        -- which part of the week, from 1; a week of one video is part 1.
--   youtube_id  -- the video's id, eleven characters of `[A-Za-z0-9_-]`.
--   recorded_at -- ISO-8601 UTC, when home recorded it.
CREATE TABLE call_part_link (
    week_from   TEXT    NOT NULL REFERENCES call_week (week_from),
    part        INTEGER NOT NULL CHECK (part BETWEEN 1 AND 99),
    youtube_id  TEXT    NOT NULL CHECK (length(youtube_id) = 11 AND youtube_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    recorded_at TEXT    NOT NULL,
    PRIMARY KEY (week_from, part, youtube_id)
);

CREATE TRIGGER call_part_link_no_update
BEFORE UPDATE ON call_part_link
BEGIN
    SELECT RAISE(ABORT, 'call_part_link is append-only: a recorded link is never rewritten');
END;

CREATE TRIGGER call_part_link_no_delete
BEFORE DELETE ON call_part_link
BEGIN
    SELECT RAISE(ABORT, 'call_part_link is append-only: a recorded link is never deleted');
END;

-- `call_part_deletion`: a receipt for each step of a part the owner let go
-- from My data, at most one per part and step, kept for good. The `rest`
-- step needs the part's `video` receipt first (checked here, as well as by
-- the repository), and the `video` step a link recorded for that part.
--
--   week_from  -- the week, as `call_week.week_from`.
--   part       -- which part, as `call_part_link.part`.
--   of         -- how many parts the week was when the step was taken: a
--                 week is let go whole once each of its `of` parts' video
--                 has a receipt, and only then is it never rendered again.
--   step       -- `video` (the part's `call.mp4`) or `rest` (the rest of its folder).
--   youtube_id -- the part's link when it was let go.
--   size       -- how many bytes went.
--   deleted_at -- ISO-8601 UTC, when home deleted them.
CREATE TABLE call_part_deletion (
    week_from  TEXT    NOT NULL REFERENCES call_week (week_from),
    part       INTEGER NOT NULL CHECK (part BETWEEN 1 AND 99),
    of         INTEGER NOT NULL CHECK (of BETWEEN 1 AND 99 AND part <= of),
    step       TEXT    NOT NULL CHECK (step IN ('video', 'rest')),
    youtube_id TEXT    NOT NULL,
    size       INTEGER NOT NULL CHECK (size >= 0),
    deleted_at TEXT    NOT NULL,
    PRIMARY KEY (week_from, part, step),
    FOREIGN KEY (week_from, part, youtube_id) REFERENCES call_part_link (week_from, part, youtube_id)
);

CREATE TRIGGER call_part_deletion_rest_after_video
BEFORE INSERT ON call_part_deletion
WHEN NEW.step = 'rest'
 AND NOT EXISTS (SELECT 1 FROM call_part_deletion WHERE week_from = NEW.week_from AND part = NEW.part AND step = 'video')
BEGIN
    SELECT RAISE(ABORT, 'call_part_deletion: the rest of a part goes only after its video');
END;

CREATE TRIGGER call_part_deletion_no_update
BEFORE UPDATE ON call_part_deletion
BEGIN
    SELECT RAISE(ABORT, 'call_part_deletion is append-only: a receipt is never rewritten');
END;

CREATE TRIGGER call_part_deletion_no_delete
BEFORE DELETE ON call_part_deletion
BEGIN
    SELECT RAISE(ABORT, 'call_part_deletion is append-only: a receipt is never deleted');
END;

-- What home recorded before parts, as part 1 of 1. Links first, which the
-- receipts name; each video receipt before its rest, which the trigger above
-- holds.
INSERT INTO call_part_link (week_from, part, youtube_id, recorded_at)
SELECT week_from, 1, youtube_id, recorded_at FROM call_video_link;

INSERT INTO call_part_deletion (week_from, part, of, step, youtube_id, size, deleted_at)
SELECT week_from, 1, 1, step, youtube_id, size, deleted_at FROM call_video_deletion WHERE step = 'video';

INSERT INTO call_part_deletion (week_from, part, of, step, youtube_id, size, deleted_at)
SELECT week_from, 1, 1, step, youtube_id, size, deleted_at FROM call_video_deletion WHERE step = 'rest';

-- The old tables are the copy now: kept whole, written no more.
CREATE TRIGGER call_video_link_frozen
BEFORE INSERT ON call_video_link
BEGIN
    SELECT RAISE(ABORT, 'call_video_link is frozen: a link is recorded in call_part_link');
END;

CREATE TRIGGER call_video_deletion_frozen
BEFORE INSERT ON call_video_deletion
BEGIN
    SELECT RAISE(ABORT, 'call_video_deletion is frozen: a receipt is written in call_part_deletion');
END;

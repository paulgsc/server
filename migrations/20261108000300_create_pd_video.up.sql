-- Playoff Delta's videos, once they are on YouTube (FEED4:
-- paulgsc/some-ui#2647), as the call's are (`call_video_link`,
-- `call_video_deletion`, ST38): home renders each voiced fill's video into
-- `renders/playoff-delta/<season>-<team>-<NN>/` on the database's drive, the
-- owner watches it on the phone, shares it to the YouTube app, records its
-- link, and then lets home's copy go from My data, in two steps: the video,
-- then the rest of its folder. The fill stays in `pd_fill`, so an episode let
-- go can always be rendered again. `pd_fill_repo::video` is the one module
-- that writes these tables.
--
-- Not subject-scoped, as `pd_fill` is not: what was said about public NFL
-- games, and its video is public on YouTube. No column names a person.
--
-- Both tables are APPEND-ONLY: the triggers abort every `UPDATE` and
-- `DELETE`. Their columns and rules are the call's, keyed by the episode
-- (`season`, `team`, `week`, as `pd_fill`'s key) in place of the week.
CREATE TABLE pd_video_link (
    season      TEXT    NOT NULL,
    team        TEXT    NOT NULL,
    week        INTEGER NOT NULL,
    youtube_id  TEXT    NOT NULL CHECK (length(youtube_id) = 11 AND youtube_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    recorded_at TEXT    NOT NULL,
    PRIMARY KEY (season, team, week, youtube_id),
    FOREIGN KEY (season, team, week) REFERENCES pd_fill (season, team, week)
);

CREATE TRIGGER pd_video_link_no_update
BEFORE UPDATE ON pd_video_link
BEGIN
    SELECT RAISE(ABORT, 'pd_video_link is append-only: a recorded link is never rewritten');
END;

CREATE TRIGGER pd_video_link_no_delete
BEFORE DELETE ON pd_video_link
BEGIN
    SELECT RAISE(ABORT, 'pd_video_link is append-only: a recorded link is never deleted');
END;

CREATE TABLE pd_video_deletion (
    season     TEXT    NOT NULL,
    team       TEXT    NOT NULL,
    week       INTEGER NOT NULL,
    step       TEXT    NOT NULL CHECK (step IN ('video', 'rest')),
    youtube_id TEXT    NOT NULL,
    size       INTEGER NOT NULL CHECK (size >= 0),
    deleted_at TEXT    NOT NULL,
    PRIMARY KEY (season, team, week, step),
    FOREIGN KEY (season, team, week, youtube_id) REFERENCES pd_video_link (season, team, week, youtube_id)
);

CREATE TRIGGER pd_video_deletion_rest_after_video
BEFORE INSERT ON pd_video_deletion
WHEN NEW.step = 'rest'
 AND NOT EXISTS (
    SELECT 1 FROM pd_video_deletion
    WHERE season = NEW.season AND team = NEW.team AND week = NEW.week AND step = 'video'
 )
BEGIN
    SELECT RAISE(ABORT, 'pd_video_deletion: the rest of an episode goes only after its video');
END;

CREATE TRIGGER pd_video_deletion_no_update
BEFORE UPDATE ON pd_video_deletion
BEGIN
    SELECT RAISE(ABORT, 'pd_video_deletion is append-only: a receipt is never rewritten');
END;

CREATE TRIGGER pd_video_deletion_no_delete
BEFORE DELETE ON pd_video_deletion
BEGIN
    SELECT RAISE(ABORT, 'pd_video_deletion is append-only: a receipt is never deleted');
END;

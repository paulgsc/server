-- Each week's Rust Rounds essay video, once it is on YouTube, and each step
-- of letting it go (RR16, paulgsc/some-ui#2691, under epic #2231), as the
-- call's and Playoff Delta's are (`call_video_*`, `pd_video_*`): home renders
-- each essay into `renders/rust-essay/<week>/` on the database's drive, the
-- owner watches it on the phone, shares it to the YouTube app, records its
-- link, and lets home's copy go from My data, the video first, then the
-- rest of its folder. The essay stays in the mailbox (`brief`), so a week
-- let go is never rendered again but can always be read.
--
-- SUBJECT-SCOPED, unlike the call's and Playoff Delta's: an essay video
-- plays the owner's own takes, so it holds the owner's recording (the one
-- named exception to docs/identity.md invariant 23; owner, 2026-11-10, via
-- the Nov 10 coordinator run; recorded on paulgsc/some-ui#2691). Both tables
-- leave with the account, and nothing here holds audio, a transcript or a
-- take's id but the deletion receipt's `note_id`.
--
-- `rust_essay_video_link`, a link per essay week and YouTube video:
--
--   subject_id  -- whose.
--   week_id     -- the essay's week (`2026-w46`).
--   youtube_id  -- the video's id on YouTube, eleven characters.
--   recorded_at -- when the owner recorded it.
--
-- `rust_essay_video_deletion`, a receipt per step of an essay week let go:
--
--   subject_id  -- whose.
--   week_id     -- the essay's week.
--   step        -- `video`, then `rest` (the rest of its folder).
--   cause       -- `let-go`: the owner let it go from My data, once its
--                  link was recorded; `take-deleted`: a take it plays was
--                  deleted from My data, so its video and folder went with
--                  it, both steps at once, linked or not.
--   youtube_id  -- the link it had, for `let-go` always; NULL for a
--                  `take-deleted` week never linked.
--   note_id     -- the deleted take, for `take-deleted`; else NULL.
--   size        -- the bytes that went.
--   deleted_at  -- when.
--
-- A week with a `video` receipt is never given to the render worker again
-- (`rust_round_repo::essay_video::let_go`).
--
-- BOTH TABLES ARE APPEND-ONLY: no row is updated, and none is deleted while
-- the subject's account stands.
CREATE TABLE rust_essay_video_link (
    subject_id  TEXT NOT NULL,
    week_id     TEXT NOT NULL,
    youtube_id  TEXT NOT NULL CHECK (length(youtube_id) = 11 AND youtube_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week_id, youtube_id)
);

CREATE TABLE rust_essay_video_deletion (
    subject_id TEXT    NOT NULL,
    week_id    TEXT    NOT NULL,
    step       TEXT    NOT NULL CHECK (step IN ('video', 'rest')),
    cause      TEXT    NOT NULL CHECK (cause IN ('let-go', 'take-deleted')),
    youtube_id TEXT,
    note_id    TEXT,
    size       INTEGER NOT NULL CHECK (size >= 0),
    deleted_at TEXT    NOT NULL,
    PRIMARY KEY (subject_id, week_id, step),
    CHECK ((cause = 'let-go' AND youtube_id IS NOT NULL AND note_id IS NULL) OR (cause = 'take-deleted' AND note_id IS NOT NULL))
);

CREATE TRIGGER rust_essay_video_deletion_rest_after_video
BEFORE INSERT ON rust_essay_video_deletion
WHEN NEW.step = 'rest'
 AND NOT EXISTS (
    SELECT 1 FROM rust_essay_video_deletion
    WHERE subject_id = NEW.subject_id AND week_id = NEW.week_id AND step = 'video'
 )
BEGIN
    SELECT RAISE(ABORT, 'rust_essay_video_deletion: the rest of an essay goes only after its video');
END;

CREATE TRIGGER rust_essay_video_deletion_let_go_needs_its_link
BEFORE INSERT ON rust_essay_video_deletion
WHEN NEW.cause = 'let-go'
 AND NOT EXISTS (
    SELECT 1 FROM rust_essay_video_link
    WHERE subject_id = NEW.subject_id AND week_id = NEW.week_id AND youtube_id = NEW.youtube_id
 )
BEGIN
    SELECT RAISE(ABORT, 'rust_essay_video_deletion: an essay is let go only once its link is recorded');
END;

CREATE TRIGGER rust_essay_video_link_is_append_only
BEFORE UPDATE ON rust_essay_video_link
BEGIN
    SELECT RAISE(ABORT, 'rust_essay_video_link is append-only: a recorded link is never rewritten');
END;

CREATE TRIGGER rust_essay_video_link_is_kept
BEFORE DELETE ON rust_essay_video_link
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'rust_essay_video_link is append-only: a link goes only with its account');
END;

CREATE TRIGGER rust_essay_video_deletion_is_append_only
BEFORE UPDATE ON rust_essay_video_deletion
BEGIN
    SELECT RAISE(ABORT, 'rust_essay_video_deletion is append-only: a receipt is never rewritten');
END;

CREATE TRIGGER rust_essay_video_deletion_is_kept
BEFORE DELETE ON rust_essay_video_deletion
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'rust_essay_video_deletion is append-only: a receipt goes only with its account');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER rust_essay_video_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM rust_essay_video_link WHERE subject_id = OLD.subject_id;
    DELETE FROM rust_essay_video_deletion WHERE subject_id = OLD.subject_id;
END;

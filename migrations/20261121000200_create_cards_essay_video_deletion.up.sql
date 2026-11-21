-- Each step of a cards essay's video going (WALK7, paulgsc/server#1055), as
-- the Rust Rounds essay's is (`rust_essay_video_deletion`, word for word but
-- for the key): a receipt per step, the video, then the rest of its folder.
--
-- Since paulgsc/server#1054 a cards essay's spoken cards play the owner's own
-- takes, so its video holds the owner's recording (docs/identity.md invariant
-- 23's essay-render exception, rule (c)). A take it plays that is deleted
-- from My data, or whose recording is let go, withdraws it: both steps'
-- receipts at once, cause `take-deleted`, naming the note beside the note's
-- own receipt, and the worker's files of it go (`cards_essays::pass::tidy`).
-- An essay with a `video` receipt is never given to the render worker again
-- and is no longer listed (`cards_essay_repo::gone`).
--
-- The `let-go` cause is the table's for paulgsc/some-ui#2999, which lets
-- home's copy go once its link is recorded; nothing writes it yet. It is in
-- the check now so that story adds a writer, not a rebuild of an append-only
-- table.
--
--   essay_key   -- the essay's key, as `cards_essay` holds it.
--   step        -- `video`, then `rest` (the rest of its folder).
--   cause       -- `take-deleted`: a take it plays went from home, so its
--                  video and folder went with it, both steps at once, linked
--                  or not; `let-go`: the owner let it go once linked.
--   subject_id  -- whose take, for `take-deleted`; NULL for `let-go`, since a
--                  cards essay is no one's.
--   youtube_id  -- the link it had: always for `let-go`; for `take-deleted`
--                  NULL when it was never linked.
--   note_id     -- the deleted take, for `take-deleted`; else NULL.
--   size        -- the bytes that went.
--   deleted_at  -- when.
--
-- APPEND-ONLY: no row is updated, and a `take-deleted` row is deleted only
-- with its subject's account, since it names the subject's note. A `let-go`
-- row names no one and is kept for good.
CREATE TABLE cards_essay_video_deletion (
    essay_key  TEXT    NOT NULL REFERENCES cards_essay (essay_key),
    step       TEXT    NOT NULL CHECK (step IN ('video', 'rest')),
    cause      TEXT    NOT NULL CHECK (cause IN ('let-go', 'take-deleted')),
    subject_id TEXT,
    youtube_id TEXT,
    note_id    TEXT,
    size       INTEGER NOT NULL CHECK (size >= 0),
    deleted_at TEXT    NOT NULL,
    PRIMARY KEY (essay_key, step),
    CHECK (
        (cause = 'let-go' AND youtube_id IS NOT NULL AND note_id IS NULL AND subject_id IS NULL)
        OR (cause = 'take-deleted' AND note_id IS NOT NULL AND subject_id IS NOT NULL)
    )
);

CREATE TRIGGER cards_essay_video_deletion_rest_after_video
BEFORE INSERT ON cards_essay_video_deletion
WHEN NEW.step = 'rest'
 AND NOT EXISTS (
    SELECT 1 FROM cards_essay_video_deletion WHERE essay_key = NEW.essay_key AND step = 'video'
 )
BEGIN
    SELECT RAISE(ABORT, 'cards_essay_video_deletion: the rest of an essay goes only after its video');
END;

CREATE TRIGGER cards_essay_video_deletion_let_go_needs_its_link
BEFORE INSERT ON cards_essay_video_deletion
WHEN NEW.cause = 'let-go'
 AND NOT EXISTS (
    SELECT 1 FROM cards_essay_link WHERE essay_key = NEW.essay_key AND youtube_id = NEW.youtube_id
 )
BEGIN
    SELECT RAISE(ABORT, 'cards_essay_video_deletion: an essay is let go only once its link is recorded');
END;

CREATE TRIGGER cards_essay_video_deletion_is_append_only
BEFORE UPDATE ON cards_essay_video_deletion
BEGIN
    SELECT RAISE(ABORT, 'cards_essay_video_deletion is append-only: a receipt is never rewritten');
END;

CREATE TRIGGER cards_essay_video_deletion_is_kept
BEFORE DELETE ON cards_essay_video_deletion
WHEN OLD.subject_id IS NULL OR EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'cards_essay_video_deletion is append-only: a receipt goes only with its account');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12): a `take-deleted` receipt names the
-- subject's note.
CREATE TRIGGER cards_essay_video_deletion_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM cards_essay_video_deletion WHERE subject_id = OLD.subject_id;
END;

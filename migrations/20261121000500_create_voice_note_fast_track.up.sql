-- The owner putting chosen takes at the front of the transcriber's line
-- (paulgsc/server#1090, "fast-track"; paulgsc/server#1091): one row per act,
-- `fast` to put the take first, `cancel` to take that back. The newest act
-- on a note decides. docs/shared-stages.md, "Who waits drives order".
--
-- A mark is ONE TRY. It is live while home has made no attempt on the note
-- since the act (`attempts`, home's count of the note's attempts when the
-- act was taken, against the count now): while live, the take is heard
-- before every other note but the one the contention guard is excusing, and
-- the lane's give does not hold it (the owner, 2026-11-21, on #1090). Once
-- tried, it only sorts ahead of notes no one waits on among notes tried as
-- often (C1017, TR1 whole), and once any model has heard the note, it is
-- spent for good: a new model hearing every note again never brings it back.
--
-- APPEND-ONLY. An act is never edited: a later act is a new row, numbered
-- after the note's last (`n`). No row is updated (the trigger below aborts
-- an `UPDATE`). The rows go with the note when it is deleted for good
-- (`voice_repo::delete`), and with the account.
--
-- Written only by `POST /api/v1/voice-notes/:id/fast-track`
-- (`voice_repo::fast_track::act`). Read by the transcriber's queue
-- (`voice_repo::transcript::waiting_first`).
--
-- Nothing here is audio or anything said: the note's id, the act and when.
--
--   subject_id, note_id -- whose take.
--   n           -- the act's number on the note, from 1.
--   act         -- `fast` or `cancel`.
--   attempts    -- how many attempts home had made on the note, by any
--                  model, when the act was taken.
--   acted_at    -- ISO-8601 UTC, when home recorded the act.
--
-- What it costs: one short row per act the owner takes.
CREATE TABLE voice_note_fast_track (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    act        TEXT    NOT NULL CHECK (act IN ('fast', 'cancel')),
    attempts   INTEGER NOT NULL CHECK (attempts >= 0),
    acted_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id, n)
);

CREATE TRIGGER voice_note_fast_track_is_append_only
BEFORE UPDATE ON voice_note_fast_track
BEGIN
    SELECT RAISE(ABORT, 'voice_note_fast_track is append-only: a later act is a new row');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER voice_note_fast_track_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note_fast_track WHERE subject_id = OLD.subject_id;
END;

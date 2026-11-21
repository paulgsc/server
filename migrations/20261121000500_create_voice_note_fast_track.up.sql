-- The owner putting chosen takes at the front of the transcriber's line
-- (paulgsc/server#1090, "fast-track"; paulgsc/server#1091): one row per act,
-- `fast` to put the take first, `cancel` to take that back. The newest act
-- on a note decides. A fast-tracked take is heard before every other note
-- but the one the contention guard is excusing, ahead of notes tried fewer
-- times (C1017), and the lane's give does not hold it (the owner,
-- 2026-11-21, on #1090). docs/shared-stages.md, "Who waits drives order".
--
-- APPEND-ONLY. An act is never edited: a later act is a new row, numbered
-- after the note's last (`n`). No row is updated (the trigger below aborts
-- an `UPDATE`). A mark outlives the take's hearing harmlessly, since a take
-- heard no longer waits; the rows go with the note when it is deleted for
-- good (`voice_repo::delete`), and with the account.
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
--   acted_at    -- ISO-8601 UTC, when home recorded the act.
--
-- What it costs: one short row per act the owner takes.
CREATE TABLE voice_note_fast_track (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    act        TEXT    NOT NULL CHECK (act IN ('fast', 'cancel')),
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

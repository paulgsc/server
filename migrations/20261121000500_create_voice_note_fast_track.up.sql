-- The owner putting chosen takes at the front of the transcriber's line
-- (paulgsc/server#1090, "fast-track"; paulgsc/server#1091): one row per act,
-- `fast` to put the take first, `cancel` to take that back. The newest act
-- on a note decides. docs/shared-stages.md, "Who waits drives order".
--
-- A mark is ONE TRY at the head of the line. While the note's newest act is
-- `fast` and the queue's model has never tried it, it is `FastTrack`: first
-- among the notes tried fewest times, which is the front of the line, and
-- the lane's give does not hold that try (the owner's ruling on #1090,
-- 2026-11-21). Once tried, or marked again after a try, it is `Wanted`:
-- ahead of notes no one waits on among notes tried as often, behind every
-- note tried fewer times, so TR1 holds literally (C1017). Once any model has
-- heard the note it is spent for good: a new model hearing every note again
-- never brings it back. No clock and no count is kept: the queue reads the
-- attempts themselves.
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

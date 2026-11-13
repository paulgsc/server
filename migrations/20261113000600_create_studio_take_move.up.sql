-- Studio takes on a conveyor (paulgsc/some-ui epic #2872, CONVEYOR1 #2873):
-- an epiphany is recorded at once and asks nothing, and lands as a draft.
-- Later, on its own Studio page, the owner moves each take along the
-- stations the owner named on 2026-11-11 (paulgsc/some-ui#2876):
--
--   in-the-oven       new: every take is here when recorded
--   cooking           being sorted
--   ready-for-pickup  the only station the routine files from
--   picked-up         filed, with its issue
--   canned            set aside and kept
--
-- Each move is its own row, and a take's station is its newest row's
-- `to_station`; a take with no row is in the oven. It is its own table, not a
-- column on `voice_note`, because a note is never updated (docs/identity.md
-- invariant 23), as `voice_note_set_aside` is beside it.
--
-- It is a record, so it is APPEND-ONLY: no move is ever updated (the trigger
-- aborts an UPDATE) and a later move is a new row. Its rows are deleted only
-- with the account, or with their take when the owner deletes it for good
-- from My data (`voice_repo::delete`, whose receipt then says the conveyor
-- history went), as the take's set-aside mark is.
--
-- Written only by `POST /api/v1/voice-notes/:id/station`
-- (`voice_repo::conveyor::advance`), and only for a `studio` take the
-- subject holds that is no idea's answer, from the station the take is at.
--
-- What a person wrote here is the owner's own labels and a short comment on
-- their own take, and the issue the routine filed it as; never audio and
-- never what was said (docs/identity.md invariant 23).
--
-- Columns of `studio_take_move`:
--
--   subject_id   -- whose record.
--   note_id      -- the take (`voice_note.note_id`), the phone's id.
--   seq          -- the move's place among the take's moves, from 1.
--   from_station -- where the take was, as the mover saw it.
--   to_station   -- where it went; never back to the oven.
--   moved_at     -- ISO-8601 UTC to the millisecond, when the owner moved it
--                   (the request's `at`).
--   labels       -- a JSON array of the owner's labels from this move on, or
--                   NULL when this move left them as they were.
--   comment      -- the owner's short comment on this move, or NULL.
--   issue        -- `<owner>/<repo>#<n>`, the issue the take was filed as:
--                   on a move to picked-up, and only there.
--   stored_at    -- ISO-8601 UTC, when home recorded the move.
CREATE TABLE studio_take_move (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    seq          INTEGER NOT NULL CHECK (seq >= 1),
    from_station TEXT    NOT NULL CHECK (from_station IN ('in-the-oven', 'cooking', 'ready-for-pickup', 'canned')),
    to_station   TEXT    NOT NULL CHECK (to_station IN ('cooking', 'ready-for-pickup', 'picked-up', 'canned')),
    moved_at     TEXT    NOT NULL,
    labels       TEXT,
    comment      TEXT,
    issue        TEXT,
    stored_at    TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id, seq),
    CHECK (from_station <> to_station),
    CHECK ((to_station = 'picked-up') = (issue IS NOT NULL)),
    CHECK (to_station <> 'picked-up' OR from_station = 'ready-for-pickup')
);

CREATE TRIGGER studio_take_move_is_append_only
BEFORE UPDATE ON studio_take_move
BEGIN
    SELECT RAISE(ABORT, 'studio_take_move is append-only: a later move is a new row');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note_set_aside` does.
CREATE TRIGGER studio_take_move_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM studio_take_move WHERE subject_id = OLD.subject_id;
END;

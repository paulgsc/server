-- Voice notes (paulgsc/server#460): the owner's voice notes, sent home by the
-- phone (paulgsc/some-ui `apps/www/src/lib/home-link`) so home can keep them
-- and transcribe them. These two tables are the one place home holds audio
-- (docs/identity.md invariant 23); the Jobs mailbox never carries it.
--
-- They are a record, not a cache, so they are APPEND-ONLY, and the audio is
-- kept for good (owner, 2026-10-25): it outlasts its transcript, since a
-- better model can hear it again, it can be cut into a montage, and takes of
-- the owner speaking Korean show how their speech changed. A note, once
-- stored, is never updated (the triggers below abort an UPDATE) or replaced:
-- `voice_repo` refuses other bytes or another description under an id it
-- holds. The only deletes are account deletion's.
--
-- The bytes sit in their own table, beside the description, so reading what
-- home holds never pulls audio, and so moving the bytes out later (to files
-- or a bucket, once they pass about 20 GB) touches one table. They are BLOBs
-- in this database rather than files: about 240 KB a minute at the phone's
-- 32 kbps, written once and read rarely, and one transaction keeps a note and
-- its bytes together, which the triggers below then hold as they hold every
-- other record (#461 has the full argument).
--
-- Columns of `voice_note`:
--
--   subject_id   -- whose note.
--   note_id      -- the phone's id for the note (a UUID today).
--   of_kind      -- what the note was said for: `reflection` (a study
--                   session's reflection), `jobs` (a Jobs pick or scout) or
--                   `scene` (a drama prompt's next scene, said aloud).
--   of_ref       -- which one: the session's id, the pick's or scout's id, or
--                   the prompt's stamp, as the phone names it.
--   recorded_at  -- when the phone recorded it, as ISO-8601 UTC to the
--                   millisecond.
--   content_type -- the audio's container, as the phone recorded it.
--   size         -- its length in bytes.
--   sha256       -- the lowercase hex SHA-256 of its bytes: what a replay is
--                   compared on, and what a later move of the bytes checks.
--   stored_at    -- ISO-8601 UTC, when it reached home.
--
-- Columns of `voice_note_audio`:
--
--   subject_id, note_id -- as above; one row per `voice_note` row.
--   body                -- the audio, byte for byte as the phone sent it.
CREATE TABLE voice_note (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene')),
    of_ref       TEXT    NOT NULL,
    recorded_at  TEXT    NOT NULL,
    content_type TEXT    NOT NULL CHECK (content_type IN ('audio/webm', 'audio/ogg', 'audio/mp4')),
    size         INTEGER NOT NULL CHECK (size > 0),
    sha256       TEXT    NOT NULL,
    stored_at    TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

CREATE TABLE voice_note_audio (
    subject_id TEXT NOT NULL,
    note_id    TEXT NOT NULL,
    body       BLOB NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

CREATE TRIGGER voice_note_is_append_only
BEFORE UPDATE ON voice_note
BEGIN
    SELECT RAISE(ABORT, 'voice_note is append-only: a note is never rewritten');
END;

CREATE TRIGGER voice_note_audio_is_append_only
BEFORE UPDATE ON voice_note_audio
BEGIN
    SELECT RAISE(ABORT, 'voice_note_audio is append-only: audio is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER voice_note_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note WHERE subject_id = OLD.subject_id;
    DELETE FROM voice_note_audio WHERE subject_id = OLD.subject_id;
END;

-- Clip frames, kept on home (SFEED, paulgsc/some-ui#2670's clips; the owner's
-- decisions of 2026-11-11, call C125). paulgsc/testing keeps no media
-- (testing#6), so home keeps each clip's frames: `file_host`'s `clips` worker
-- reads `clips/index.json` on a timer, renders a walk side itself on its
-- render lane, fetches the six legacy PNGs held to pinned SHA-256s, and keeps
-- nothing of a session-taken side but the labels the index already holds.
-- `clip_frame_repo` is the one module that writes this table.
--
-- The PNG is on the drive, under `renders/clips/<clip_id>/<side>/<file>`,
-- never in a row: a row says what home holds and that it checked it.
--
-- Not subject-scoped: a frame is a screenshot of an app on sample data, the
-- index public. No column names a person.
--
-- APPEND-ONLY: the triggers abort every `UPDATE` and `DELETE`. A frame is
-- kept once, by (clip_id, side, file); other bytes under a held name are
-- refused by `clip_frame_repo::keep`, never written.
--
-- Columns:
--
--   clip_id  -- the clip's id in the index (`studio-release-promo`).
--   side     -- `before` or `after`.
--   file     -- the frame's file name, one path segment (`01.png`).
--   made     -- `walk` (home rendered it) or `legacy` (fetched, digest pinned).
--   sha256   -- lowercase hex SHA-256 of the PNG.
--   size     -- its length in bytes, at most 1 MiB.
--   taken_at -- ISO-8601 UTC, when home kept it.
CREATE TABLE clip_frame (
    clip_id  TEXT    NOT NULL CHECK (length(clip_id) BETWEEN 1 AND 80 AND clip_id NOT GLOB '*[^a-z0-9-]*'),
    side     TEXT    NOT NULL CHECK (side IN ('before', 'after')),
    file     TEXT    NOT NULL CHECK (length(file) BETWEEN 5 AND 80 AND file NOT GLOB '*[^a-z0-9._-]*' AND file GLOB '[a-z0-9]*.png'),
    made     TEXT    NOT NULL CHECK (made IN ('walk', 'legacy')),
    sha256   TEXT    NOT NULL CHECK (length(sha256) = 64),
    size     INTEGER NOT NULL CHECK (size BETWEEN 1 AND 1048576),
    taken_at TEXT    NOT NULL,
    PRIMARY KEY (clip_id, side, file)
);

CREATE TRIGGER clip_frame_no_update
BEFORE UPDATE ON clip_frame
BEGIN
    SELECT RAISE(ABORT, 'clip_frame is append-only: a kept frame is never rewritten');
END;

CREATE TRIGGER clip_frame_no_delete
BEFORE DELETE ON clip_frame
BEGIN
    SELECT RAISE(ABORT, 'clip_frame is append-only: a kept frame is never deleted');
END;

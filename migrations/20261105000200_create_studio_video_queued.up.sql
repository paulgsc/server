-- A finished video's DNA, queued on home (STUDIO ST33: paulgsc/some-ui#2591,
-- the owner's decision 9 on paulgsc/some-ui#2248, 2026-11-04). A video is its
-- DNA: the template it goes through, the shape the owner built in Studio, the
-- takes it uses and the data that fills it. Finishing a video on the phone
-- queues the DNA here as one record per revision.
--
-- What home checks, what it keeps and who reads it are docs/identity.md
-- invariant 36's. In short: one row per revision queued, APPEND-ONLY (the
-- trigger aborts an UPDATE; the only deletes are account deletion's,
-- invariant 12), the DNA kept byte for byte, and a take carries only its
-- note id.
--
-- Columns of `studio_video_queued`, one row per revision queued:
--
--   subject_id       -- whose record.
--   video_id         -- the phone's id for the video (its project's id).
--   rev              -- the revision of the video queued, from 1.
--   title            -- the DNA's title, trimmed: 1 to 200 characters.
--   queued_at        -- when the owner finished it, as the DNA's `queuedAt`
--                       says, ISO-8601 UTC to the millisecond.
--   template_kind    -- the DNA's `template.kind` (`show-format` today).
--   template_id      -- the DNA's `template.id`, the template's name.
--   template_version -- the DNA's `template.version`, from 1.
--   dna              -- the DNA's JSON, byte for byte as the phone sent it.
--   dna_sha256       -- the SHA-256 of `dna`, lowercase hex: a put of the
--                       same revision compares on it.
--   stored_at        -- ISO-8601 UTC, when it reached home.
CREATE TABLE studio_video_queued (
    subject_id       TEXT    NOT NULL,
    video_id         TEXT    NOT NULL,
    rev              INTEGER NOT NULL CHECK (rev > 0),
    title            TEXT    NOT NULL,
    queued_at        TEXT    NOT NULL,
    template_kind    TEXT    NOT NULL,
    template_id      TEXT    NOT NULL,
    template_version INTEGER NOT NULL CHECK (template_version > 0),
    dna              TEXT    NOT NULL,
    dna_sha256       TEXT    NOT NULL,
    stored_at        TEXT    NOT NULL,
    PRIMARY KEY (subject_id, video_id, rev)
);

-- The list: a subject's queued videos, newest queued first. The primary key
-- serves a put's lookup by video and revision.
CREATE INDEX studio_video_queued_order ON studio_video_queued (subject_id, queued_at);

CREATE TRIGGER studio_video_queued_is_append_only
BEFORE UPDATE ON studio_video_queued
BEGIN
    SELECT RAISE(ABORT, 'studio_video_queued is append-only: queue a new revision');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER studio_video_queued_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM studio_video_queued WHERE subject_id = OLD.subject_id;
END;

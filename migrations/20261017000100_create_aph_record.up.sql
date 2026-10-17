-- aph (paulgsc/some-ui `packages/ui/aph`): the owner's own reconciliation
-- log, a figure of mine against a figure reported by the other side, at fixed
-- checkpoints of the day. Until now it lived in one `localStorage` key on the
-- phone and had no other copy. Once the person opts in on the phone, these
-- two tables are its record and the phone keeps only what it has not yet
-- sent.
--
-- It is a record, not a convenience copy (compare `learner_shelf`'s point 5),
-- so it is APPEND-ONLY. Every write adds a revision; no row is ever updated
-- (the trigger below aborts an UPDATE), and the only deletes are account
-- deletion's. An entry as it stands is its highest revision, and every
-- earlier one is kept: what was written down, and when, is the evidence the
-- record exists for.
--
-- What a revision may change is held by `aph_repo` (and, on the phone, by
-- `breaches` in `packages/ui/aph/src/lib/model.ts`, the same rules): an entry
-- never disappears; its day, checkpoint, clock time and goal never change;
-- their figure, once reported, is never replaced or removed. And of the
-- entries at one checkpoint on one day, at most one is plain (carries no
-- label), which is what `plain` and `aph_entry_slot` are for.
--
-- Columns of `aph_entry`:
--
--   subject_id  -- whose record.
--   entry_id    -- the phone's id for the entry; a URL path segment, held to
--                  `curriculum_repo::is_plain_key`'s rule.
--   revision    -- 1 for the entry as first written, then 2, 3 … per change.
--   day         -- the entry's local calendar day, `YYYY-MM-DD`. Also in
--                  `body`; a column so the plain-slot check is an indexed read.
--   checkpoint  -- the checkpoint id, or NULL off the checkpoints. Likewise.
--   plain       -- 1 when this revision carries no label. Likewise.
--   body        -- the entry's JSON as `aph_repo` re-serialised it after
--                  checking it, which is what a read serves.
--   written_at  -- ISO-8601 UTC, when this revision was written. The one
--                  instant stored; docs/identity.md, "What is still exposed".
CREATE TABLE aph_entry (
    subject_id TEXT    NOT NULL,
    entry_id   TEXT    NOT NULL,
    revision   INTEGER NOT NULL CHECK (revision >= 1),
    day        TEXT    NOT NULL,
    checkpoint TEXT,
    plain      INTEGER NOT NULL CHECK (plain IN (0, 1)),
    body       TEXT    NOT NULL,
    written_at TEXT    NOT NULL,
    PRIMARY KEY (subject_id, entry_id, revision)
);

-- The plain-slot check: the entries at one checkpoint on one day.
CREATE INDEX aph_entry_slot ON aph_entry (subject_id, day, checkpoint);

-- The settings the entries are read against (checkpoints and their goals,
-- labels, tolerance …), as one JSON body per revision, append-only alike.
CREATE TABLE aph_settings (
    subject_id TEXT    NOT NULL,
    revision   INTEGER NOT NULL CHECK (revision >= 1),
    body       TEXT    NOT NULL,
    written_at TEXT    NOT NULL,
    PRIMARY KEY (subject_id, revision)
);

CREATE TRIGGER aph_entry_is_append_only
BEFORE UPDATE ON aph_entry
BEGIN
    SELECT RAISE(ABORT, 'aph_entry is append-only: write a new revision');
END;

CREATE TRIGGER aph_settings_is_append_only
BEFORE UPDATE ON aph_settings
BEGIN
    SELECT RAISE(ABORT, 'aph_settings is append-only: write a new revision');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `learner_shelf` does.
CREATE TRIGGER aph_entry_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM aph_entry WHERE subject_id = OLD.subject_id;
END;

CREATE TRIGGER aph_settings_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM aph_settings WHERE subject_id = OLD.subject_id;
END;

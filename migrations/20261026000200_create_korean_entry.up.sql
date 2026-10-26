-- Korean's record (paulgsc/some-ui#2100, KO-HOME): the word sets the owner
-- studies and what the phone records as they read them (each word's pace,
-- each sentence's tally, the lens record, the practice record, the set left
-- unfinished, the answers at a sitting's end). Until now all of it lived in
-- the phone's `localStorage`, and word sets stopped at 50. The owner decided
-- (2026-10-26, some-ui#2098) that home is the store: once the phone is opted
-- in, this table is the record and the phone keeps only what home has not
-- yet confirmed, plus an offline set its person curates.
--
-- It is a record, not a convenience copy (compare `learner_shelf`'s point
-- 5), so it is APPEND-ONLY, as `aph_entry` is. Every write adds a revision;
-- no row is ever updated (the trigger below aborts an UPDATE), and the only
-- deletes are account deletion's. Removing an entry is a revision too, one
-- whose body is NULL, so what was read, and when, stays.
--
-- Content only, never parsed: home checks that a body is one JSON object or
-- array within `korean_repo::BODY_CEILING`, and stores its bytes verbatim.
-- What a word set or a pace *is* stays the phone's to check.
--
-- Columns of `korean_entry`:
--
--   subject_id  -- whose record.
--   kind        -- which of the phone's stores the entry is from:
--                  `korean_repo::Kind`, a closed list kept in code (`set`,
--                  `pace`, `tally`, `lens`, `record`, `progress`,
--                  `feedback`), so a new kind is a reviewed diff there.
--   entry_key   -- the phone's key for the entry within its kind: a URL
--                  path segment, held to `curriculum_repo::is_plain_key`.
--   revision    -- 1 for the entry as first written, then 2, 3 … per change.
--   body        -- the entry's JSON as the phone sent it, byte for byte, or
--                  NULL for a revision that removed it.
--   sha256      -- lowercase hex SHA-256 over `body`'s bytes, NULL with it:
--                  how a write of the bytes already standing is told apart.
--   written_at  -- ISO-8601 UTC, when this revision was written. The one
--                  instant stored; docs/identity.md, "What is still exposed".
CREATE TABLE korean_entry (
    subject_id TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    entry_key  TEXT    NOT NULL,
    revision   INTEGER NOT NULL CHECK (revision >= 1),
    body       TEXT,
    sha256     TEXT,
    written_at TEXT    NOT NULL,
    CHECK ((body IS NULL) = (sha256 IS NULL)),
    PRIMARY KEY (subject_id, kind, entry_key, revision)
);

CREATE TRIGGER korean_entry_is_append_only
BEFORE UPDATE ON korean_entry
BEGIN
    SELECT RAISE(ABORT, 'korean_entry is append-only: write a new revision');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `aph_entry` does.
CREATE TRIGGER korean_entry_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM korean_entry WHERE subject_id = OLD.subject_id;
END;

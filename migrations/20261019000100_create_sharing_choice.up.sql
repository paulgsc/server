-- What a paired phone's person chose to keep sending to home, and when that
-- began and ended (paulgsc/some-ui `apps/www/src/lib/home-link`, its consent
-- log): one row per change of a choice, for one pairing and one kind of
-- record. The phone's own log is the record of what was agreed to, in its
-- words; this is home's, so home can refuse what a phone sends after its
-- person stopped it, and show when each choice started and stopped.
--
-- Two times per choice, never one per item sent: home keeps when sharing
-- began and ended, and nothing about when anything was sent under it
-- (docs/identity.md, invariant 17). APPEND-ONLY: a row is never updated (the
-- trigger below aborts an UPDATE) or deleted but with the account.
--
-- Columns of `sharing_choice`:
--
--   subject_id -- whose account.
--   device_id  -- the pairing it was made under: a digest of that pairing's
--                 session (`subject::PairedDevice`), never anything the
--                 phone says about itself.
--   seq        -- this pairing's changes, in the order home heard them.
--   scope      -- what is shared: `sessions`, `shelf`, `aph` or `jobs`.
--   kept       -- 1 when sharing began, 0 when it stopped.
--   at         -- ISO-8601 UTC, when home heard it.
CREATE TABLE sharing_choice (
    subject_id TEXT    NOT NULL,
    device_id  TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    scope      TEXT    NOT NULL CHECK (scope IN ('sessions', 'shelf', 'aph', 'jobs')),
    kept       INTEGER NOT NULL CHECK (kept IN (0, 1)),
    at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, device_id, seq)
);
CREATE TRIGGER sharing_choice_is_append_only
BEFORE UPDATE ON sharing_choice
BEGIN
    SELECT RAISE(ABORT, 'sharing_choice is append-only: write a new change');
END;
-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER sharing_choice_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM sharing_choice WHERE subject_id = OLD.subject_id;
END;

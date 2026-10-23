-- Home's archive of the Jobs mailbox (paulgsc/some-ui `apps/mailbox`,
-- "Archiving"). The mailbox is where the cloud routine and home meet, not
-- where the record lives: a file live there long enough moves home, and the
-- mailbox keeps only a row about it. Home stores the bytes here first and
-- only then asks the mailbox to archive them, so nothing is ever held by
-- neither. When the routine asks for an archived file again, home puts these
-- bytes back.
--
-- APPEND-ONLY, as the rest of home's copy is (invariant 19): no row is
-- updated (the trigger aborts an `UPDATE`), nothing writes `INSERT OR
-- REPLACE`, and nothing deletes but account deletion. A name holds one body,
-- as in the mailbox: a file archived again after being put back is already
-- here.
--
-- Bodies are JSON files, never audio (docs/identity.md invariants 16 and 19).
--
-- Columns, one row per file home archived:
--
--   subject_id -- whose copy (the configured `MAILBOX_SUBJECT`).
--   folder     -- the mailbox folder: `brief`, `decisions` or `scout`.
--   name       -- its name there.
--   seq        -- the mailbox's sequence number for it when home first
--                 archived it.
--   sha256     -- lowercase hex SHA-256 of `body`, as the mailbox listed it.
--   size       -- `body`'s length in bytes.
--   put_at     -- ISO-8601 UTC, when it was first put in the mailbox.
--   body       -- the file's bytes, exactly as the mailbox held them.
--   kept_at    -- ISO-8601 UTC, when home stored it here.
CREATE TABLE mailbox_archive (
    subject_id TEXT NOT NULL,
    folder     TEXT NOT NULL,
    name       TEXT NOT NULL,
    seq        INTEGER NOT NULL,
    sha256     TEXT NOT NULL,
    size       INTEGER NOT NULL,
    put_at     TEXT NOT NULL,
    body       BLOB NOT NULL,
    kept_at    TEXT NOT NULL,
    PRIMARY KEY (subject_id, folder, name)
);

CREATE TRIGGER mailbox_archive_is_append_only
BEFORE UPDATE ON mailbox_archive
BEGIN
    SELECT RAISE(ABORT, 'mailbox_archive is append-only: an archived file is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `mailbox_item` does.
CREATE TRIGGER mailbox_archive_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_archive WHERE subject_id = OLD.subject_id;
END;

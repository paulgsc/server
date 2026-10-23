-- The Jobs mailbox (paulgsc/some-ui `apps/mailbox`): a Cloudflare Worker
-- where the owner's cloud routine and home leave each other files, since
-- neither can reach the other (home is LAN-only and only calls out). The
-- routine puts each day's digest in the `brief` folder; home reads it from
-- there so the phone can take today's picks from home. Later (Stage 4), home
-- hands the phone's voice-free decisions and scouts to the routine through
-- the `decisions` and `scout` folders.
--
-- These tables are home's copy of that traffic, for the one subject the
-- deployment names in `MAILBOX_SUBJECT`. They are a record, so they are
-- APPEND-ONLY, as the mailbox itself is: no row is ever updated (each
-- table's trigger aborts an `UPDATE`) and nothing deletes but account
-- deletion. Never write a row with `INSERT OR REPLACE`: SQLite runs no
-- `UPDATE` trigger for it, so it would replace a row these triggers exist to
-- keep (the mailbox's own README, MB1, names the same gap).
--
-- Bodies are JSON files, never audio (docs/identity.md invariants 16 and 19).
--
-- Columns of `mailbox_item`, one row per file home has read from the mailbox:
--
--   subject_id -- whose copy (the configured `MAILBOX_SUBJECT`).
--   folder     -- the mailbox folder it came from: `brief` today.
--   name       -- its name there, e.g. `digest-2026-10-22.json`.
--   seq        -- the mailbox's sequence number for it. The next read asks
--                 for items after the highest `seq` held per folder, so there
--                 is no cursor to keep anywhere else.
--   sha256     -- lowercase hex SHA-256 of `body`, as the mailbox listed it.
--   size       -- `body`'s length in bytes.
--   body       -- the file's bytes, exactly as the mailbox held them.
--   pulled_at  -- ISO-8601 UTC, when home read it.
CREATE TABLE mailbox_item (
    subject_id TEXT NOT NULL,
    folder     TEXT NOT NULL,
    name       TEXT NOT NULL,
    seq        INTEGER NOT NULL,
    sha256     TEXT NOT NULL,
    size       INTEGER NOT NULL,
    body       BLOB NOT NULL,
    pulled_at  TEXT NOT NULL,
    PRIMARY KEY (subject_id, folder, name)
);

-- The cursor read: the highest `seq` held per (subject, folder).
CREATE INDEX mailbox_item_seq ON mailbox_item (subject_id, folder, seq);

-- Columns of `mailbox_outgoing` (Stage 4), one row per file the phone filed
-- with home to be handed to the mailbox:
--
--   subject_id -- whose file.
--   folder     -- `decisions` or `scout`.
--   name       -- the file's name, which the mailbox will hold it under.
--   sha256     -- lowercase hex SHA-256 of `body`.
--   body       -- the file's bytes, as the phone sent them.
--   filed_at   -- ISO-8601 UTC, when the phone filed it.
CREATE TABLE mailbox_outgoing (
    subject_id TEXT NOT NULL,
    folder     TEXT NOT NULL,
    name       TEXT NOT NULL,
    sha256     TEXT NOT NULL,
    body       BLOB NOT NULL,
    filed_at   TEXT NOT NULL,
    PRIMARY KEY (subject_id, folder, name)
);

-- Columns of `mailbox_handed` (Stage 4), one row each time home handed an
-- outgoing file to the mailbox and heard an answer it keeps:
--
--   subject_id -- whose file.
--   folder     -- as in `mailbox_outgoing`.
--   name       -- as in `mailbox_outgoing`.
--   answer     -- what the mailbox said: it took the file (`added`), already
--                 held these bytes (`unchanged`), holds other bytes under the
--                 name (`conflict`), or refused the file (`refused`).
--   handed_at  -- ISO-8601 UTC, when it answered.
CREATE TABLE mailbox_handed (
    subject_id TEXT NOT NULL,
    folder     TEXT NOT NULL,
    name       TEXT NOT NULL,
    answer     TEXT NOT NULL CHECK (answer IN ('added', 'unchanged', 'conflict', 'refused')),
    handed_at  TEXT NOT NULL
);

CREATE INDEX mailbox_handed_file ON mailbox_handed (subject_id, folder, name);

CREATE TRIGGER mailbox_item_is_append_only
BEFORE UPDATE ON mailbox_item
BEGIN
    SELECT RAISE(ABORT, 'mailbox_item is append-only: a held file is never rewritten');
END;

CREATE TRIGGER mailbox_outgoing_is_append_only
BEFORE UPDATE ON mailbox_outgoing
BEGIN
    SELECT RAISE(ABORT, 'mailbox_outgoing is append-only: a filed file is never rewritten');
END;

CREATE TRIGGER mailbox_handed_is_append_only
BEFORE UPDATE ON mailbox_handed
BEGIN
    SELECT RAISE(ABORT, 'mailbox_handed is append-only: write a new answer');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER mailbox_item_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_item WHERE subject_id = OLD.subject_id;
END;

CREATE TRIGGER mailbox_outgoing_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_outgoing WHERE subject_id = OLD.subject_id;
END;

CREATE TRIGGER mailbox_handed_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_handed WHERE subject_id = OLD.subject_id;
END;

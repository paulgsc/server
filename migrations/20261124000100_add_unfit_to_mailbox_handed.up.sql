-- A file the mailbox refused for what it is, not for what the mailbox holds
-- (server#1172, epic #1174): a name that cannot be put in a URL, or a body over
-- the mailbox's ceiling (a `413`). Neither changes for the same file, so the
-- answer is final, `unfit`, and never handed again; only `refused` (a `422`,
-- the mailbox's own words, which a fixed mailbox may take) is handed again
-- (`mailbox_repo::pending_outgoing`, MI8, server#1192). Every reader that asks
-- whether the mailbox holds a file reads `answer IN ('added', 'unchanged')`,
-- so `unfit` is not taken by any of them.
--
-- SQLite cannot alter a CHECK, so this rebuilds the table the way
-- `20261025000300_add_voice_to_sharing_choice.up.sql` rebuilt
-- `sharing_choice`: a fresh `mailbox_handed_new` with the wider CHECK, every
-- row copied across unchanged, the old table dropped and the new one renamed
-- in. The table is append-only, so the copy takes every row and changes none;
-- the columns and their order are the original's, and only the CHECK on
-- `answer` widens.
--
-- Columns are as `20261023000100_create_mailbox_copy.up.sql` describes them;
-- `answer` is now `added`, `unchanged`, `conflict`, `refused` or `unfit`.
--
-- The index and both triggers are the original's, recreated word for word.
-- The index and the append-only trigger live on `mailbox_handed` and go with
-- the dropped table. The account trigger lives on `account` and names
-- `mailbox_handed` in its body, so it is dropped first and recreated once the
-- rebuilt table holds the name again. Nothing else was created on this table.
DROP TRIGGER mailbox_handed_leaves_with_account;

CREATE TABLE mailbox_handed_new (
    subject_id TEXT NOT NULL,
    folder     TEXT NOT NULL,
    name       TEXT NOT NULL,
    answer     TEXT NOT NULL CHECK (answer IN ('added', 'unchanged', 'conflict', 'refused', 'unfit')),
    handed_at  TEXT NOT NULL
);

INSERT INTO mailbox_handed_new (subject_id, folder, name, answer, handed_at)
SELECT subject_id, folder, name, answer, handed_at FROM mailbox_handed ORDER BY rowid;

DROP TABLE mailbox_handed;
ALTER TABLE mailbox_handed_new RENAME TO mailbox_handed;

CREATE INDEX mailbox_handed_file ON mailbox_handed (subject_id, folder, name);

CREATE TRIGGER mailbox_handed_is_append_only
BEFORE UPDATE ON mailbox_handed
BEGIN
    SELECT RAISE(ABORT, 'mailbox_handed is append-only: write a new answer');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER mailbox_handed_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_handed WHERE subject_id = OLD.subject_id;
END;

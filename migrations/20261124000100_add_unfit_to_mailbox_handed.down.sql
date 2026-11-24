-- Reverses 20261124000100_add_unfit_to_mailbox_handed.up.sql: rebuilds
-- `mailbox_handed` back to the four answers
-- `20261023000100_create_mailbox_copy.up.sql` allowed, the same way.
--
-- **It refuses rather than discards.** Every row is copied, an `unfit` one
-- included, into a table whose CHECK refuses `unfit`; so once any file has
-- been answered `unfit`, this fails on that row and the migration's
-- transaction rolls back, leaving the table as it was. The rows are
-- append-only, so rolling back past them is a decision for a person, never a
-- side effect of a down migration.
--
-- As in the up migration, the account trigger names this table, so it goes
-- first and comes back last.
DROP TRIGGER mailbox_handed_leaves_with_account;

CREATE TABLE mailbox_handed_old (
    subject_id TEXT NOT NULL,
    folder     TEXT NOT NULL,
    name       TEXT NOT NULL,
    answer     TEXT NOT NULL CHECK (answer IN ('added', 'unchanged', 'conflict', 'refused')),
    handed_at  TEXT NOT NULL
);

INSERT INTO mailbox_handed_old (subject_id, folder, name, answer, handed_at)
SELECT subject_id, folder, name, answer, handed_at FROM mailbox_handed ORDER BY rowid;

DROP TABLE mailbox_handed;
ALTER TABLE mailbox_handed_old RENAME TO mailbox_handed;

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

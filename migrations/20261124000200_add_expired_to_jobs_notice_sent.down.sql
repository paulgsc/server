-- Reverses 20261124000200_add_expired_to_jobs_notice_sent.up.sql: rebuilds
-- `jobs_notice_sent` back to the three values
-- `20261025000400_create_jobs_notice.up.sql` allowed, the same way.
--
-- **It refuses rather than discards.** Every row is copied, an `expired` one
-- included, into a table whose CHECK refuses `expired`; so once any record
-- has been stored `expired`, this fails on that row and the migration's
-- transaction rolls back, leaving the table as it was. The rows are
-- append-only, so rolling back past them is a decision for a person, never a
-- side effect of a down migration.
--
-- As in the up migration, the account trigger names this table, so it goes
-- first and comes back last.
DROP TRIGGER jobs_notice_leaves_with_account;

CREATE TABLE jobs_notice_sent_old (
    subject_id TEXT    NOT NULL,
    run_name   TEXT    NOT NULL,
    said       TEXT    NOT NULL CHECK (said IN ('picks', 'trouble', 'skipped')),
    devices    INTEGER NOT NULL CHECK (devices >= 0),
    at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, run_name)
);

INSERT INTO jobs_notice_sent_old (subject_id, run_name, said, devices, at)
SELECT subject_id, run_name, said, devices, at FROM jobs_notice_sent ORDER BY rowid;

DROP TABLE jobs_notice_sent;
ALTER TABLE jobs_notice_sent_old RENAME TO jobs_notice_sent;

CREATE TRIGGER jobs_notice_sent_is_append_only
BEFORE UPDATE ON jobs_notice_sent
BEGIN
    SELECT RAISE(ABORT, 'jobs_notice_sent is append-only: what home said is never rewritten');
END;

CREATE TRIGGER jobs_notice_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM jobs_notice_device WHERE subject_id = OLD.subject_id;
    DELETE FROM jobs_notice_sent WHERE subject_id = OLD.subject_id;
END;

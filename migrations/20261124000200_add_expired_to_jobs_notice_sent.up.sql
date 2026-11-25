-- A Jobs run record no phone was told of before it left the notice's window
-- (`jobs_notice::WINDOW_HOURS`, 12 hours) is a loss (server#1176, epic #1174):
-- after server#1206 a record every phone refused, or one FCM never answered,
-- is left unsent and tried again each pass, and one the window outlives is
-- never told. The owner's rule is that a failure is never destroyed, so the
-- loss is counted once (`jobs-notice`/`untold`), and to count it once it is
-- stored: `said = 'expired'`, `devices = 0`, written by the first pass that
-- finds the record past its window and unnamed here. Like every row here it
-- names the record, so it is never written twice and never tried again.
--
-- SQLite cannot alter a CHECK, so this rebuilds the table the way
-- `20261124000100_add_unfit_to_mailbox_handed.up.sql` rebuilt
-- `mailbox_handed`: a fresh `jobs_notice_sent_new` with the wider CHECK,
-- every row copied across unchanged, the old table dropped and the new one
-- renamed in. The table is append-only, so the copy takes every row and
-- changes none; the columns, their order and the primary key are the
-- original's, and only the CHECK on `said` widens.
--
-- Columns are as `20261025000400_create_jobs_notice.up.sql` describes them;
-- `said` is now `picks`, `trouble`, `skipped` or `expired`.
--
-- Both triggers are the original's, recreated word for word. The append-only
-- trigger lives on `jobs_notice_sent` and goes with the dropped table. The
-- account trigger lives on `account` and names both of the notice's tables in
-- its body, so it is dropped first and recreated once the rebuilt table holds
-- the name again. Nothing else was created on this table.
DROP TRIGGER jobs_notice_leaves_with_account;

CREATE TABLE jobs_notice_sent_new (
    subject_id TEXT    NOT NULL,
    run_name   TEXT    NOT NULL,
    said       TEXT    NOT NULL CHECK (said IN ('picks', 'trouble', 'skipped', 'expired')),
    devices    INTEGER NOT NULL CHECK (devices >= 0),
    at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, run_name)
);

INSERT INTO jobs_notice_sent_new (subject_id, run_name, said, devices, at)
SELECT subject_id, run_name, said, devices, at FROM jobs_notice_sent ORDER BY rowid;

DROP TABLE jobs_notice_sent;
ALTER TABLE jobs_notice_sent_new RENAME TO jobs_notice_sent;

CREATE TRIGGER jobs_notice_sent_is_append_only
BEFORE UPDATE ON jobs_notice_sent
BEGIN
    SELECT RAISE(ABORT, 'jobs_notice_sent is append-only: what home said is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note` does.
CREATE TRIGGER jobs_notice_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM jobs_notice_device WHERE subject_id = OLD.subject_id;
    DELETE FROM jobs_notice_sent WHERE subject_id = OLD.subject_id;
END;

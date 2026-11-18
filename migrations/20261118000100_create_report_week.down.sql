-- Reverses 20261118000100_create_report_week.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261103001700_create_app_event.down.sql does: `report_week` is
-- append-only, so once it holds a row this fails on the CHECK below and the
-- migration's transaction rolls back, leaving the table as it was.
CREATE TEMP TABLE report_week_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO report_week_is_empty (n) SELECT COUNT(*) FROM report_week;
DROP TABLE report_week_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER report_week_leaves_with_account;
DROP TRIGGER report_week_is_kept;
DROP TRIGGER report_week_is_append_only;
DROP TABLE report_week;

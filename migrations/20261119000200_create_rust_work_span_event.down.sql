-- Reverses 20261119000200_create_rust_work_span_event.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261109000100_create_rust_week_record.down.sql does: the table is the
-- owner's record of their work, append-only, so once it holds a row this
-- fails on the CHECK below and the migration's transaction rolls back,
-- leaving the table as it was.
CREATE TEMP TABLE rust_work_span_event_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO rust_work_span_event_is_empty (n) SELECT COUNT(*) FROM rust_work_span_event;
DROP TABLE rust_work_span_event_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER rust_work_span_event_leaves_with_account;
DROP TRIGGER rust_work_span_event_is_kept;
DROP TRIGGER rust_work_span_event_is_append_only;
DROP TRIGGER rust_work_span_event_closes_a_start;
DROP TRIGGER rust_work_span_event_one_open;
DROP INDEX rust_work_span_event_by_week;
DROP INDEX rust_work_span_event_by_kind;
DROP TABLE rust_work_span_event;

-- Reverses 20261109000100_create_rust_week_record.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261108075000_create_episode_note.down.sql does: the tables are the
-- owner's record of their weeks and their card-era sessions, append-only, so
-- once any holds a row this fails on the CHECK below and the migration's
-- transaction rolls back, leaving the tables as they were.
CREATE TEMP TABLE rust_week_record_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO rust_week_record_is_empty (n)
    SELECT (SELECT COUNT(*) FROM rust_week_sent) + (SELECT COUNT(*) FROM rust_week_paste) + (SELECT COUNT(*) FROM rust_round_log);
DROP TABLE rust_week_record_is_empty;

-- The account trigger lives on `account`, so dropping the tables would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER rust_week_record_leaves_with_account;
DROP TRIGGER rust_round_log_is_kept;
DROP TRIGGER rust_round_log_is_append_only;
DROP TRIGGER rust_week_paste_is_kept;
DROP TRIGGER rust_week_paste_is_append_only;
DROP TRIGGER rust_week_sent_is_kept;
DROP TRIGGER rust_week_sent_is_append_only;
DROP TABLE rust_round_log;
DROP TABLE rust_week_paste;
DROP TABLE rust_week_sent;

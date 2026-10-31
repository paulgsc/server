-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER rust_round_leaves_with_account;
DROP TRIGGER rust_round_drop_is_kept;
DROP TRIGGER rust_round_drop_is_append_only;
DROP TRIGGER rust_round_run_is_kept;
DROP TRIGGER rust_round_run_is_append_only;
DROP TRIGGER rust_round_source_is_kept;
DROP TRIGGER rust_round_source_is_append_only;
DROP TRIGGER rust_round_is_kept;
DROP TRIGGER rust_round_is_append_only;
DROP TABLE rust_round_drop;
DROP TABLE rust_round_run;
DROP TABLE rust_round_source;
DROP INDEX rust_round_by_subject;
DROP TABLE rust_round;

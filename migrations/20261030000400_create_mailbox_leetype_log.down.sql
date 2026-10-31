-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER mailbox_leetype_log_leaves_with_account;
DROP TRIGGER mailbox_leetype_log_is_append_only;
DROP TABLE mailbox_leetype_log;

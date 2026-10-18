-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER jobs_event_leaves_with_account;
DROP TRIGGER jobs_event_is_append_only;
DROP INDEX jobs_event_order;
DROP TABLE jobs_event;

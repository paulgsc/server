-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER leetype_session_event_leaves_with_account;
DROP TRIGGER leetype_session_event_is_append_only;
DROP INDEX leetype_session_event_order;
DROP TABLE leetype_session_event;

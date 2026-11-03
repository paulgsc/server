-- Reverses 20261103001700_create_app_event.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261030000200_create_studio_log.down.sql does: `app_event` is
-- append-only, so once it holds a row this fails on the CHECK below and the
-- migration's transaction rolls back, leaving the table as it was. Rolling
-- back past this migration with events in it is a decision for a person
-- (give them a durable copy first), never a side effect of a down migration.
CREATE TEMP TABLE app_event_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO app_event_is_empty (n) SELECT COUNT(*) FROM app_event;
DROP TABLE app_event_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER app_event_leaves_with_account;
DROP TRIGGER app_event_is_append_only;
DROP INDEX app_event_by_family;
DROP TABLE app_event;

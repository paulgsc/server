-- Reverses 20261030000200_create_studio_log.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261026000200_create_korean_entry.down.sql does: both tables are the
-- owner's Studio record, append-only, so once either holds a row this fails
-- on the CHECK below and the migration's transaction rolls back, leaving the
-- tables as they were. Rolling back past this migration with a record in it
-- is a decision for a person (give them a durable copy first), never a side
-- effect of a down migration.
CREATE TEMP TABLE studio_log_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO studio_log_is_empty (n) SELECT COUNT(*) FROM studio_sitting_event;
INSERT INTO studio_log_is_empty (n) SELECT COUNT(*) FROM studio_friction;
DROP TABLE studio_log_is_empty;

-- The account triggers live on `account`, so dropping the tables would leave
-- them behind, failing every later account deletion. They go first.
DROP TRIGGER studio_friction_leaves_with_account;
DROP TRIGGER studio_friction_is_append_only;
DROP INDEX studio_friction_order;
DROP TABLE studio_friction;
DROP TRIGGER studio_sitting_event_leaves_with_account;
DROP TRIGGER studio_sitting_event_is_append_only;
DROP INDEX studio_sitting_event_order;
DROP TABLE studio_sitting_event;

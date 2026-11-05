-- Reverses 20261105000200_create_studio_video_queued.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261030000200_create_studio_log.down.sql does: `studio_video_queued` is
-- the owner's record, append-only, so once it holds a row this fails on the
-- CHECK below and the migration's transaction rolls back, leaving the table
-- as it was. Rolling back past this migration with a queued video in it is a
-- decision for a person (give them a durable copy first), never a side
-- effect of a down migration.
CREATE TEMP TABLE studio_video_queued_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO studio_video_queued_is_empty (n) SELECT COUNT(*) FROM studio_video_queued;
DROP TABLE studio_video_queued_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER studio_video_queued_leaves_with_account;
DROP TRIGGER studio_video_queued_is_append_only;
DROP INDEX studio_video_queued_order;
DROP TABLE studio_video_queued;

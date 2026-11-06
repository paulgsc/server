-- Reverses 20261106000100_create_call_week.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261105000200_create_studio_video_queued.down.sql does: `call_week` is the
-- only place older weeks of the call are kept once the repository lets them
-- go, so once it holds a row this fails on the CHECK below and the
-- migration's transaction rolls back, leaving the table as it was. Rolling
-- back past this migration with a week in it is a decision for a person
-- (give them a durable copy first), never a side effect of a down migration.
CREATE TEMP TABLE call_week_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO call_week_is_empty (n) SELECT COUNT(*) FROM call_week;
DROP TABLE call_week_is_empty;

DROP TRIGGER call_week_no_delete;
DROP TRIGGER call_week_no_update;
DROP TABLE call_week;

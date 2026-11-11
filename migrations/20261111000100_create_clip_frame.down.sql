-- Reverses 20261111000100_create_clip_frame.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261106000100_create_call_week.down.sql does: once `clip_frame` holds a
-- row this fails on the CHECK below and the migration's transaction rolls
-- back. Rolling back past this migration with a frame in it is a decision for
-- a person, never a side effect of a down migration.
CREATE TEMP TABLE clip_frame_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO clip_frame_is_empty (n) SELECT COUNT(*) FROM clip_frame;
DROP TABLE clip_frame_is_empty;

DROP TRIGGER clip_frame_no_delete;
DROP TRIGGER clip_frame_no_update;
DROP TABLE clip_frame;

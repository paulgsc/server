-- Reverses 20261114000200_create_call_part.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261106000100_create_call_week.down.sql does: once either new table holds
-- a row the old tables lack (anything recorded since, a part past the first
-- above all), this fails on the CHECK below and the migration's transaction
-- rolls back. The rows copied from the old tables are still there, in the
-- old tables, so dropping their copies loses nothing.
CREATE TEMP TABLE call_part_is_copy (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO call_part_is_copy (n)
SELECT COUNT(*) FROM call_part_link AS part
WHERE part.part <> 1
   OR NOT EXISTS (SELECT 1 FROM call_video_link AS old
                  WHERE old.week_from = part.week_from AND old.youtube_id = part.youtube_id);
INSERT INTO call_part_is_copy (n)
SELECT COUNT(*) FROM call_part_deletion AS part
WHERE part.part <> 1
   OR NOT EXISTS (SELECT 1 FROM call_video_deletion AS old
                  WHERE old.week_from = part.week_from AND old.step = part.step);
DROP TABLE call_part_is_copy;

DROP TRIGGER call_video_deletion_frozen;
DROP TRIGGER call_video_link_frozen;
DROP TRIGGER call_part_deletion_no_delete;
DROP TRIGGER call_part_deletion_no_update;
DROP TRIGGER call_part_deletion_rest_after_video;
DROP TABLE call_part_deletion;
DROP TRIGGER call_part_link_no_delete;
DROP TRIGGER call_part_link_no_update;
DROP TABLE call_part_link;

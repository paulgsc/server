-- Reverses 20261118000200_create_report_video.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261118000100_create_report_week.down.sql does: both tables are
-- append-only, so once either holds a row this fails on the CHECK below and
-- the migration's transaction rolls back, leaving them as they were.
CREATE TEMP TABLE report_video_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO report_video_is_empty (n)
SELECT (SELECT COUNT(*) FROM report_video_link) + (SELECT COUNT(*) FROM report_video_deletion);
DROP TABLE report_video_is_empty;

-- The account trigger lives on `account`, so dropping the tables would
-- leave it behind, failing every later account deletion. It goes first.
DROP TRIGGER report_video_leaves_with_account;
DROP TRIGGER report_video_deletion_is_kept;
DROP TRIGGER report_video_deletion_is_append_only;
DROP TRIGGER report_video_link_is_kept;
DROP TRIGGER report_video_link_is_append_only;
DROP TRIGGER report_video_deletion_rest_after_video;
DROP TRIGGER report_video_deletion_needs_its_link;
DROP TRIGGER report_video_link_names_a_week;
DROP TABLE report_video_deletion;
DROP TABLE report_video_link;

-- Reverses 20261110000200_create_rust_essay_video.up.sql.
--
-- **It refuses rather than discards:** a link or a receipt is kept for
-- good, so once either table holds a row this fails on the CHECK below and
-- the migration's transaction rolls back, leaving both as they were.
CREATE TEMP TABLE rust_essay_video_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO rust_essay_video_is_empty (n)
    SELECT (SELECT COUNT(*) FROM rust_essay_video_link) + (SELECT COUNT(*) FROM rust_essay_video_deletion);
DROP TABLE rust_essay_video_is_empty;

-- The account trigger lives on `account`, so it goes before the tables.
DROP TRIGGER rust_essay_video_leaves_with_account;
DROP TABLE rust_essay_video_deletion;
DROP TABLE rust_essay_video_link;

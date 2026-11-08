-- Reverses 20261108000300_create_pd_video.up.sql, refusing rather than
-- discarding once either table holds a row, as pd_fill's down migration does:
-- a link or a receipt is the owner's record.
CREATE TEMP TABLE pd_video_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO pd_video_is_empty (n) SELECT (SELECT COUNT(*) FROM pd_video_link) + (SELECT COUNT(*) FROM pd_video_deletion);
DROP TABLE pd_video_is_empty;

DROP TABLE pd_video_deletion;
DROP TABLE pd_video_link;

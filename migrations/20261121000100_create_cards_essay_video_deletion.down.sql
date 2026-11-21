-- Reverses 20261121000100_create_cards_essay_video_deletion.up.sql.
--
-- **It refuses rather than discards:** a receipt is kept for good, so once
-- the table holds a row this fails on the CHECK below and the migration's
-- transaction rolls back, leaving it as it was.
CREATE TEMP TABLE cards_essay_video_deletion_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO cards_essay_video_deletion_is_empty (n) SELECT COUNT(*) FROM cards_essay_video_deletion;
DROP TABLE cards_essay_video_deletion_is_empty;

-- The account trigger lives on `account`, so it goes before the table.
DROP TRIGGER cards_essay_video_deletion_leaves_with_account;
DROP TABLE cards_essay_video_deletion;

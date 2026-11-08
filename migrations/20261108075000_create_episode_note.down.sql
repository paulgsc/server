-- Reverses 20261108075000_create_episode_note.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261101000100_create_written_reflection.down.sql does: the table is the
-- owner's record of their notes on the feeds' episodes, append-only, so once
-- it holds a row this fails on the CHECK below and the migration's
-- transaction rolls back, leaving the table as it was.
CREATE TEMP TABLE episode_note_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO episode_note_is_empty (n) SELECT COUNT(*) FROM episode_note;
DROP TABLE episode_note_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER episode_note_leaves_with_account;
DROP TRIGGER episode_note_is_append_only;
DROP TRIGGER episode_note_categories_are_the_set;
DROP INDEX episode_note_by_feed;
DROP TABLE episode_note;

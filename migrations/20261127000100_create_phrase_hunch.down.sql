-- Reverses 20261127000100_create_phrase_hunch.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261126000100_create_conversation_event.down.sql does: the table is the
-- owner's history of the hunches no phrase did, append-only, so once it holds
-- a row this fails on the CHECK below and the migration's transaction rolls
-- back, leaving the table as it was.
CREATE TEMP TABLE phrase_hunch_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO phrase_hunch_is_empty (n) SELECT COUNT(*) FROM phrase_hunch;
DROP TABLE phrase_hunch_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER phrase_hunch_leaves_with_account;
DROP TRIGGER phrase_hunch_is_append_only;
DROP TABLE phrase_hunch;

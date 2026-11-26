-- Reverses 20261126000100_create_conversation_event.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261108075000_create_episode_note.down.sql does: the table is the owner's
-- record of their conversations, append-only, so once it holds a row this
-- fails on the CHECK below and the migration's transaction rolls back,
-- leaving the table as it was.
CREATE TEMP TABLE conversation_event_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO conversation_event_is_empty (n) SELECT COUNT(*) FROM conversation_event;
DROP TABLE conversation_event_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER conversation_event_leaves_with_account;
DROP TRIGGER conversation_event_is_append_only;
DROP TRIGGER conversation_event_is_next;
DROP TABLE conversation_event;

-- Reverses 20261113000600_create_studio_take_move.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261101000900_create_voice_note_set_aside.down.sql does: the table is
-- home's record of how the owner moved each take, append-only, so once it
-- holds a row this fails on the CHECK below and the migration's transaction
-- rolls back, leaving the table as it was.
CREATE TEMP TABLE studio_take_move_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO studio_take_move_is_empty (n) SELECT COUNT(*) FROM studio_take_move;
DROP TABLE studio_take_move_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER studio_take_move_leaves_with_account;
DROP TRIGGER studio_take_move_is_append_only;
DROP TABLE studio_take_move;

-- Reverses 20261103000100_create_voice_note_deletion.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261101000900_create_voice_note_set_aside.down.sql does: a receipt is
-- never deleted, so once the table holds a row this fails on the CHECK below
-- and the migration's transaction rolls back, leaving the table as it was.
CREATE TEMP TABLE voice_note_deletion_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO voice_note_deletion_is_empty (n) SELECT COUNT(*) FROM voice_note_deletion;
DROP TABLE voice_note_deletion_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER voice_note_deletion_leaves_with_account;
DROP TRIGGER voice_note_deletion_is_append_only;
DROP TABLE voice_note_deletion;

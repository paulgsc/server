-- Reverses 20261101000900_create_voice_note_set_aside.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261101000800_create_studio_take_on_drive.down.sql does: the table is
-- home's record of which notes the owner set aside, append-only, so once it
-- holds a row this fails on the CHECK below and the migration's transaction
-- rolls back, leaving the table as it was. Rolling back past this migration
-- with a mark in it is a decision for a person, never a side effect of a
-- down migration.
CREATE TEMP TABLE voice_note_set_aside_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO voice_note_set_aside_is_empty (n) SELECT COUNT(*) FROM voice_note_set_aside;
DROP TABLE voice_note_set_aside_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER voice_note_set_aside_leaves_with_account;
DROP TRIGGER voice_note_set_aside_is_append_only;
DROP TABLE voice_note_set_aside;

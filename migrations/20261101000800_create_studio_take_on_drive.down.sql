-- Reverses 20261101000800_create_studio_take_on_drive.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261101000700_create_drama_prompt.down.sql does: the table is home's
-- record of which takes Drive held, append-only, so once it holds a row
-- this fails on the CHECK below and the migration's transaction rolls back,
-- leaving the table as it was. Rolling back past this migration with a
-- record in it is a decision for a person, never a side effect of a down
-- migration.
CREATE TEMP TABLE studio_take_on_drive_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO studio_take_on_drive_is_empty (n) SELECT COUNT(*) FROM studio_take_on_drive;
DROP TABLE studio_take_on_drive_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER studio_take_on_drive_leaves_with_account;
DROP TRIGGER studio_take_on_drive_is_append_only;
DROP TABLE studio_take_on_drive;

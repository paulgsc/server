-- Reverses 20261101000600_create_study_note.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261026000200_create_korean_entry.down.sql does: the table is the owner's
-- record, append-only, so once it holds a row this fails on the CHECK below
-- and the migration's transaction rolls back, leaving the table as it was.
-- Rolling back past this migration with a record in it is a decision for a
-- person (give them a durable copy first), never a side effect of a down
-- migration.
CREATE TEMP TABLE study_note_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO study_note_is_empty (n) SELECT COUNT(*) FROM study_note;
DROP TABLE study_note_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER study_note_leaves_with_account;
DROP TRIGGER study_note_is_append_only;
DROP TABLE study_note;

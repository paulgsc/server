-- Reverses 20261101000700_create_drama_prompt.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261101000100_create_written_reflection.down.sql does: the table is the
-- owner's record of the drama prompts they asked for, append-only, so once
-- it holds a row this fails on the CHECK below and the migration's
-- transaction rolls back, leaving the table as it was. Rolling back past
-- this migration with a record in it is a decision for a person (give them a
-- durable copy first), never a side effect of a down migration.
CREATE TEMP TABLE drama_prompt_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO drama_prompt_is_empty (n) SELECT COUNT(*) FROM drama_prompt;
DROP TABLE drama_prompt_is_empty;

-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER drama_prompt_leaves_with_account;
DROP TRIGGER drama_prompt_is_append_only;
DROP TABLE drama_prompt;

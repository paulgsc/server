-- Reverses 20261114000100_create_cards_essay.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261106000100_create_call_week.down.sql does: once `cards_essay` holds a
-- row this fails on the CHECK below and the migration's transaction rolls
-- back. Rolling back past this migration with an essay in it is a decision
-- for a person, never a side effect of a down migration.
CREATE TEMP TABLE cards_essay_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO cards_essay_is_empty (n) SELECT COUNT(*) FROM cards_essay;
DROP TABLE cards_essay_is_empty;

DROP TRIGGER cards_essay_no_delete;
DROP TRIGGER cards_essay_no_update;
DROP TABLE cards_essay;

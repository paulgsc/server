-- Reverses 20261108000200_create_pd_fill.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261106000100_create_call_week.down.sql does: `pd_fill` is home's record
-- of what was said each week, with the season it was said over, so once it
-- holds a row this fails on the CHECK below and the migration's transaction
-- rolls back, leaving the table as it was. Rolling back past this migration
-- with a fill in it is a decision for a person (give them a durable copy
-- first), never a side effect of a down migration.
CREATE TEMP TABLE pd_fill_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO pd_fill_is_empty (n) SELECT COUNT(*) FROM pd_fill;
DROP TABLE pd_fill_is_empty;

DROP TRIGGER pd_fill_no_delete;
DROP TRIGGER pd_fill_no_update;
DROP TABLE pd_fill;

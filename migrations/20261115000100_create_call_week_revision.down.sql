-- Reverses 20261115000100_create_call_week_revision.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261106000100_create_call_week.down.sql does: a revision is the only copy
-- of what the repository served for it, so once the table holds a row this
-- fails on the CHECK below and the migration's transaction rolls back.
CREATE TEMP TABLE call_week_revision_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO call_week_revision_is_empty (n) SELECT COUNT(*) FROM call_week_revision;
DROP TABLE call_week_revision_is_empty;

DROP TRIGGER call_week_revision_no_delete;
DROP TRIGGER call_week_revision_no_update;
DROP TRIGGER call_week_revision_in_order;
DROP TABLE call_week_revision;

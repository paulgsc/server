-- Reverses 20261101002600_create_idea.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261030000200_create_studio_log.down.sql does: every table is the owner's
-- record, append-only, so once any holds a row this fails on the CHECK below
-- and the migration's transaction rolls back, leaving the tables as they
-- were. Rolling back past this migration with a record in it is a decision
-- for a person (give them a durable copy first), never a side effect of a
-- down migration.
CREATE TEMP TABLE idea_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO idea_is_empty (n) SELECT COUNT(*) FROM idea_revision;
INSERT INTO idea_is_empty (n) SELECT COUNT(*) FROM idea_answer;
INSERT INTO idea_is_empty (n) SELECT COUNT(*) FROM idea_furnishing;
INSERT INTO idea_is_empty (n) SELECT COUNT(*) FROM idea_set_aside;
INSERT INTO idea_is_empty (n) SELECT COUNT(*) FROM idea_list_mark;
DROP TABLE idea_is_empty;

-- The account triggers live on `account`, so dropping the tables would leave
-- them behind, failing every later account deletion. They go first.
DROP TRIGGER idea_list_mark_leaves_with_account;
DROP TRIGGER idea_list_mark_is_append_only;
DROP TABLE idea_list_mark;
DROP TRIGGER idea_set_aside_leaves_with_account;
DROP TRIGGER idea_set_aside_is_append_only;
DROP TABLE idea_set_aside;
DROP TRIGGER idea_furnishing_leaves_with_account;
DROP TRIGGER idea_furnishing_is_append_only;
DROP TABLE idea_furnishing;
DROP TRIGGER idea_answer_leaves_with_account;
DROP TRIGGER idea_answer_is_append_only;
DROP TABLE idea_answer;
DROP TRIGGER idea_revision_leaves_with_account;
DROP TRIGGER idea_revision_is_append_only;
DROP INDEX idea_revision_created;
DROP TABLE idea_revision;

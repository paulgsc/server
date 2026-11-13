-- Reverses 20261113003000_create_interview_week_draft_final.up.sql.
--
-- **It refuses rather than discards,** as the Rust Rounds draft and final's
-- down migration does: once any week is final, this fails on the CHECK below
-- and the migration's transaction rolls back, leaving both tables as they
-- were. A draft is the phone's too, so dropping drafts loses nothing.
CREATE TEMP TABLE interview_week_final_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO interview_week_final_is_empty (n) SELECT COUNT(*) FROM interview_week_final;
DROP TABLE interview_week_final_is_empty;

-- The account trigger lives on `account`, so it goes before the tables.
DROP TRIGGER interview_week_draft_final_leave_with_account;
DROP TRIGGER interview_week_final_is_kept;
DROP TRIGGER interview_week_final_is_append_only;
DROP TABLE interview_week_final;
DROP TABLE interview_week_draft;

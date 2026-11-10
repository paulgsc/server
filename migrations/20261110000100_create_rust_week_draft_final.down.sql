-- Reverses 20261110000100_create_rust_week_draft_final.up.sql.
--
-- **It refuses rather than discards,** as
-- 20261109000100_create_rust_week_record.down.sql does: once a week is final,
-- or a draft holds a send that is not a copy of a `rust_week_sent` row (the
-- up migration's seed), this fails on the CHECK below and the migration's
-- transaction rolls back, leaving both tables as they were.
CREATE TEMP TABLE rust_week_draft_final_is_seed_only (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO rust_week_draft_final_is_seed_only (n)
    SELECT (SELECT COUNT(*) FROM rust_week_final)
         + (SELECT COUNT(*) FROM rust_week_draft AS d
            WHERE NOT EXISTS (SELECT 1 FROM rust_week_sent AS s
                              WHERE s.subject_id = d.subject_id AND s.week_id = d.week_id
                                AND s.sent_at = d.sent_at AND s.body = d.body));
DROP TABLE rust_week_draft_final_is_seed_only;

-- The account trigger lives on `account`, so it goes before the tables.
DROP TRIGGER rust_week_draft_final_leave_with_account;
DROP TRIGGER rust_week_final_is_kept;
DROP TRIGGER rust_week_final_is_append_only;
DROP TABLE rust_week_final;
DROP TABLE rust_week_draft;

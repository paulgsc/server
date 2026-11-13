-- A week of mock interview is a draft until its deadline, and only what
-- stands at the deadline is final (paulgsc/some-ui#2935, IW4, under epic
-- #2931), as a week of Rust Rounds is (RR22,
-- `20261110000100_create_rust_week_draft_final.up.sql`, whose rules these
-- follow word for word).
--
-- The week itself is the routine's: `interview-week-<id>.json` in the
-- mailbox's `brief` folder, kept in `mailbox_item` as every `brief` file is.
-- What the owner did with it is theirs: which questions they answered and
-- which teleprompter cards they read, each with which take (a voice note, by
-- id, never audio) and when it was recorded, and which they passed over.
--
-- The phone may send a week again and again before its `due`, each send
-- replacing the last as the week's draft (`interview_week_draft`); at `due`
-- the draft standing then becomes the week's final (`interview_week_final`),
-- and only the final is the record. A send that reaches home after `due` by
-- home's clock is refused (`422`), so the draft standing at `due` can change
-- no more, and home writes it as final in one transaction at the first read
-- of the week after `due` (`GET /interview-weeks/:id/record`, a send refused
-- for it, or the mailbox pass that puts the interview index). Not answering
-- by the deadline is final too: a week with no final names every question
-- unanswered and every card unread.
--
-- The logic is Rust Rounds', not a copy of it: `rust_round_repo::draft`
-- keeps both kinds' drafts and finals, and only the table each statement
-- names differs.
--
-- `interview_week_draft`, one row per (subject, week) not yet final:
--
--   subject_id  -- whose.
--   week_id     -- the week (`2026-w47`), as the mailbox's week names it.
--   sent_at     -- when the owner pressed Send (RFC 3339), never after `due`.
--   body        -- the phone's send, checked and re-serialised
--                  (`rust_round_repo::interview::Sent`). Never audio.
--   received_at -- ISO-8601 UTC, when it reached home.
--
-- Replaced by each newer send of the week (a later `sent_at`), and deleted in
-- the transaction that writes it as final. NOT append-only and not kept for
-- good: the phone holds the draft too, and only the final is the record.
--
-- `interview_week_final`, one row per (subject, week) whose draft stood at
-- `due`: the same columns, the draft as it stood. The record: never updated
-- (the trigger below aborts it) and never deleted while the subject's
-- account stands.
--
-- Both are subject-scoped, read only by their subject, and leave with the
-- account (docs/identity.md invariant 28).
CREATE TABLE interview_week_draft (
    subject_id  TEXT NOT NULL,
    week_id     TEXT NOT NULL,
    sent_at     TEXT NOT NULL,
    body        TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week_id)
);

CREATE TABLE interview_week_final (
    subject_id  TEXT NOT NULL,
    week_id     TEXT NOT NULL,
    sent_at     TEXT NOT NULL,
    body        TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week_id)
);

CREATE TRIGGER interview_week_final_is_append_only
BEFORE UPDATE ON interview_week_final
BEGIN
    SELECT RAISE(ABORT, 'interview_week_final is append-only: what stood at the deadline is kept as it was');
END;

CREATE TRIGGER interview_week_final_is_kept
BEFORE DELETE ON interview_week_final
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'interview_week_final is append-only: a week final goes only with its account');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER interview_week_draft_final_leave_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM interview_week_draft WHERE subject_id = OLD.subject_id;
    DELETE FROM interview_week_final WHERE subject_id = OLD.subject_id;
END;

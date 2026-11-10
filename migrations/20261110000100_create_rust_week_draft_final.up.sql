-- A week of Rust Rounds is a draft until its deadline, and only what stands
-- at the deadline is final (RR22, paulgsc/some-ui#2832, under epic #2231).
-- The owner, 2026-11-10: "what we need is a 'draft' state mechanism, where
-- only the deadline lock really counts as 'final'. So what is sent or not
-- sent at deadline is what is subject to invariant."
--
-- RR14 (paulgsc/server#763) kept a week's Send once, in `rust_week_sent`, and
-- refused another. Now the phone may send a week again and again before its
-- `due`, each send replacing the last as the week's draft
-- (`rust_week_draft`); at `due` the draft standing then becomes the week's
-- final (`rust_week_final`), and only the final is the record. A send that
-- reaches home after `due` by home's clock is refused (`422`), even one
-- pressed before it (no grace, owner, 2026-11-10), so the draft standing at
-- `due` can change no more, and writing it as final is the same whenever it
-- happens: home does it in one transaction at the first read of the week
-- after `due` (`GET /rust-weeks/:id/record`, `GET /subjects/me/kept`, or a
-- send refused for it). Not answering by the deadline is final too: a week
-- with no final names every ticket unanswered.
--
-- **`rust_week_sent` is left exactly as it is,** rows and triggers: RR14's
-- archive, kept for good, never written after this. One row is held on the
-- owner's home (2026-w46, before its due), so each of its rows is copied
-- below as its week's standing draft; the original stays where it is.
--
-- `rust_week_draft`, one row per (subject, week) not yet final:
--
--   subject_id  -- whose.
--   week_id     -- the week (`2026-w46`), as the mailbox's week names it.
--   sent_at     -- when the owner pressed Send (RFC 3339), never after `due`.
--   body        -- the phone's `WeekSent`, checked and re-serialised, as in
--                  `rust_week_sent`. Never audio: a take is named by id.
--   received_at -- ISO-8601 UTC, when it reached home.
--
-- Replaced by each newer send of the week (a later `sent_at`), and deleted in
-- the transaction that writes it as final. NOT append-only and not kept for
-- good: the phone holds the draft too, and only the final is the record.
--
-- `rust_week_final`, one row per (subject, week) whose draft stood at `due`:
-- the same columns, the draft as it stood, `received_at` when that send
-- reached home. The
-- record: never updated (the trigger below aborts it) and never deleted
-- while the subject's account stands.
--
-- Both are subject-scoped, read only by their subject, and leave with the
-- account (docs/identity.md invariant 28).
CREATE TABLE rust_week_draft (
    subject_id  TEXT NOT NULL,
    week_id     TEXT NOT NULL,
    sent_at     TEXT NOT NULL,
    body        TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week_id)
);

CREATE TABLE rust_week_final (
    subject_id  TEXT NOT NULL,
    week_id     TEXT NOT NULL,
    sent_at     TEXT NOT NULL,
    body        TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week_id)
);

CREATE TRIGGER rust_week_final_is_append_only
BEFORE UPDATE ON rust_week_final
BEGIN
    SELECT RAISE(ABORT, 'rust_week_final is append-only: what stood at the deadline is kept as it was');
END;

CREATE TRIGGER rust_week_final_is_kept
BEFORE DELETE ON rust_week_final
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'rust_week_final is append-only: a week final goes only with its account');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12). Its own trigger, so RR14's is left as it is.
CREATE TRIGGER rust_week_draft_final_leave_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM rust_week_draft WHERE subject_id = OLD.subject_id;
    DELETE FROM rust_week_final WHERE subject_id = OLD.subject_id;
END;

-- Each week RR14 kept is its week's standing draft, copied: read as final at
-- the first read after its `due`, like any other.
INSERT INTO rust_week_draft (subject_id, week_id, sent_at, body, received_at)
    SELECT subject_id, week_id, sent_at, body, received_at FROM rust_week_sent;

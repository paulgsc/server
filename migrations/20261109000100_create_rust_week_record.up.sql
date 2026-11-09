-- Each week of Rust Rounds kept on home as one record, silence included
-- (paulgsc/some-ui#2689, RR14, under epic #2231), and the card-era log the
-- phone kept before the weeks (`some-ui.rust-round-log/1`, which RR7 #2238
-- meant to send home and never did).
--
-- A week is the routine's (`mailbox_item`, RR12). What the owner did with it
-- is theirs: which tickets they answered, with which take, and when they
-- sent them (`rust_week_sent`), and the error they pasted from their own
-- code that week (`rust_week_paste`). Not answering is an answer: a ticket
-- the week names and no take answers is a silence, and home says so in the
-- week's record (`GET /rust-weeks/:id/record`) without anything sent. How
-- often each ticket was opened is never kept here: it is read from the
-- phone's engagement events in `app_event` (CK2, ENG4), so there is one
-- counter, not two.
--
-- All three are subject-scoped, read only by their subject, and leave with
-- the account (docs/identity.md invariant 28). ALL THREE ARE APPEND-ONLY:
-- no row is updated (the triggers below abort it), and none is deleted while
-- the subject's account stands. Another body under a key the subject holds
-- is refused (`409`), never written over, except that a card-era session
-- that grew is kept as a new revision beside the one before.
--
-- `rust_week_sent`, one row per week the owner sent:
--
--   subject_id  -- whose.
--   week_id     -- the week (`2026-w46`), as the mailbox's week names it.
--   sent_at     -- when the owner pressed Send on the phone, as the phone
--                  said it (RFC 3339). Never after the week's `due`: a take
--                  not sent by the deadline doesn't count (owner,
--                  2026-11-07), so home refuses it.
--   body        -- the phone's `WeekSent` (`{week, at, answers, passed}`),
--                  checked and re-serialised: each ticket answered, its
--                  take's voice-note id and when it was recorded, and the
--                  tickets passed over. Never audio: a take is named by id.
--   received_at -- ISO-8601 UTC, when it reached home.
CREATE TABLE rust_week_sent (
    subject_id  TEXT NOT NULL,
    week_id     TEXT NOT NULL,
    sent_at     TEXT NOT NULL,
    body        TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week_id)
);

-- `rust_week_paste`, one row per error the owner pasted in a week and kept
-- (paulgsc/some-ui#2733, "Paste an error"):
--
--   subject_id  -- whose.
--   week_id     -- the week it was pasted in.
--   at          -- when the paste was opened on the phone (RFC 3339), which
--                  names it within its week, and in its voice note's ref
--                  (`<week>:PASTE:<at>`).
--   text        -- the error, as pasted (at most 20,000 characters): the
--                  owner's own code's output, so it can hold anything their
--                  code printed.
--   note_id     -- the voice note said on it, by id, or NULL.
--   received_at -- ISO-8601 UTC, when it reached home.
CREATE TABLE rust_week_paste (
    subject_id  TEXT NOT NULL,
    week_id     TEXT NOT NULL,
    at          TEXT NOT NULL,
    text        TEXT NOT NULL,
    note_id     TEXT,
    received_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, week_id, at)
);

-- `rust_round_log`, one row per revision of a card-era session:
--
--   subject_id  -- whose.
--   session_id  -- the phone's id for the session.
--   revision    -- 1 for the session as first sent, one more each time it
--                  arrives grown: every list it holds starts with what the
--                  revision before held.
--   started_at  -- when the session started: the phone's `startedAt` as
--                  UTC to the millisecond when it is an RFC 3339 instant
--                  (so text order is time order, for MIN and ORDER BY),
--                  else as the phone wrote it. The body keeps its text.
--   body        -- the session, byte for byte as the phone sent it (JSON
--                  whitespace around it trimmed): the cards played, what was
--                  picked, which notes were said on them (by id), what rustc
--                  printed, the pins, the errors pasted from the desk and
--                  the cards the tutor guessed wrong. Read, never
--                  re-serialised.
--   sha256      -- the hex SHA-256 of `body`.
--   written_at  -- ISO-8601 UTC, when it reached home.
CREATE TABLE rust_round_log (
    subject_id TEXT    NOT NULL,
    session_id TEXT    NOT NULL,
    revision   INTEGER NOT NULL CHECK (revision >= 1),
    started_at TEXT    NOT NULL,
    body       TEXT    NOT NULL,
    sha256     TEXT    NOT NULL,
    written_at TEXT    NOT NULL,
    PRIMARY KEY (subject_id, session_id, revision)
);

CREATE TRIGGER rust_week_sent_is_append_only
BEFORE UPDATE ON rust_week_sent
BEGIN
    SELECT RAISE(ABORT, 'rust_week_sent is append-only: a week is sent once');
END;

CREATE TRIGGER rust_week_sent_is_kept
BEFORE DELETE ON rust_week_sent
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'rust_week_sent is append-only: a week sent goes only with its account');
END;

CREATE TRIGGER rust_week_paste_is_append_only
BEFORE UPDATE ON rust_week_paste
BEGIN
    SELECT RAISE(ABORT, 'rust_week_paste is append-only: a paste is never rewritten');
END;

CREATE TRIGGER rust_week_paste_is_kept
BEFORE DELETE ON rust_week_paste
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'rust_week_paste is append-only: a paste goes only with its account');
END;

CREATE TRIGGER rust_round_log_is_append_only
BEFORE UPDATE ON rust_round_log
BEGIN
    SELECT RAISE(ABORT, 'rust_round_log is append-only: a session grows by a new revision');
END;

CREATE TRIGGER rust_round_log_is_kept
BEFORE DELETE ON rust_round_log
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'rust_round_log is append-only: a session goes only with its account');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER rust_week_record_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM rust_week_sent WHERE subject_id = OLD.subject_id;
    DELETE FROM rust_week_paste WHERE subject_id = OLD.subject_id;
    DELETE FROM rust_round_log WHERE subject_id = OLD.subject_id;
END;

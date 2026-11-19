-- The owner's work on a week of Rust Rounds, kept on home as spans of time
-- (paulgsc/some-ui#3250, RR26). The owner works on a ticket's steps (read,
-- academy, voice) mostly away from the phone; on the phone they start a work
-- tracker, which stops itself after two hours, and when they come back the
-- phone asks whether they were working the whole time: all of it, part of
-- it (so many minutes), or none. Each span is evidence of the owner's work
-- over time, so it is kept for good.
--
-- `rust_work_span_event`, one row per event of a span: its start, and its
-- close once the phone has the owner's answer.
--
--   subject_id      -- whose.
--   span_id         -- the phone's id for the span, a UUID it made.
--   kind            -- `started` or `closed`.
--   at              -- when it happened on the phone (RFC 3339): when the
--                      tracker started, or when it stopped (by the owner, or
--                      by itself), as the phone wrote it.
--   week_id         -- started only: the week (`2026-w47`).
--   ticket          -- started only: the ticket's key (`TB-101`).
--   step            -- started only: `read`, `academy` or `voice`.
--   exercise        -- started only: the step's exercise, from 1, or NULL.
--   points          -- started only: the speak-to points worked on, a JSON
--                      array of distinct integers from 1, never empty.
--   device          -- started only: the phone's own name for the device
--                      that started it (at most 64 characters), so another
--                      device can tell whose open span it sees. A label the
--                      phone gives itself: never a key, never an identity
--                      (docs/identity.md invariant 4).
--   stopped_by      -- closed only: `you` (the owner stopped it) or `itself`
--                      (the two hours ran out).
--   answer          -- closed only: `all`, `part` or `none`.
--   counted_seconds -- closed only: the seconds the owner says they worked,
--                      at most the span's whole seconds (closed `at` less
--                      started `at`, rounded down); all of them for `all`,
--                      0 for `none`.
--   kept_at         -- ISO-8601 UTC, when it reached home.
--
-- Subject-scoped, read only by its subject, and leaves with the account
-- (docs/identity.md invariants 12 and 28). APPEND-ONLY: a span is never
-- updated (the trigger below aborts it), and no row is deleted while the
-- subject's account stands. A span's start and its close are each written
-- once: the same event again changes nothing, another under the same key is
-- refused (`409`), never written over.
--
-- **One span open at a time per subject.** A span is open from its start
-- until its close. A start while one is open is refused, and so is a close
-- with no start; the triggers below hold both, whatever writes. Since no
-- span starts while another is open, the open span, if any, is always the
-- subject's newest start, which `rust_work_span_event_by_kind` reads in one
-- step however many spans the subject holds.
CREATE TABLE rust_work_span_event (
    subject_id      TEXT    NOT NULL,
    span_id         TEXT    NOT NULL,
    kind            TEXT    NOT NULL CHECK (kind IN ('started', 'closed')),
    at              TEXT    NOT NULL,
    week_id         TEXT,
    ticket          TEXT,
    step            TEXT    CHECK (step IN ('read', 'academy', 'voice')),
    exercise        INTEGER CHECK (exercise >= 1),
    points          TEXT,
    device          TEXT,
    stopped_by      TEXT    CHECK (stopped_by IN ('you', 'itself')),
    answer          TEXT    CHECK (answer IN ('all', 'part', 'none')),
    counted_seconds INTEGER CHECK (counted_seconds >= 0),
    kept_at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, span_id, kind),
    CHECK (
        (kind = 'started'
            AND week_id IS NOT NULL AND ticket IS NOT NULL AND step IS NOT NULL
            AND points IS NOT NULL AND device IS NOT NULL
            AND stopped_by IS NULL AND answer IS NULL AND counted_seconds IS NULL)
        OR (kind = 'closed'
            AND week_id IS NULL AND ticket IS NULL AND step IS NULL AND exercise IS NULL
            AND points IS NULL AND device IS NULL
            AND stopped_by IS NOT NULL AND answer IS NOT NULL AND counted_seconds IS NOT NULL
            AND (answer <> 'none' OR counted_seconds = 0))
    )
);

-- A subject's starts (or closes) in the order they reached home: an index's
-- entries for one key are in rowid order, so the newest start is its last.
CREATE INDEX rust_work_span_event_by_kind ON rust_work_span_event (subject_id, kind);

-- A week's spans, for `GET /rust-work-spans?week=`.
CREATE INDEX rust_work_span_event_by_week ON rust_work_span_event (subject_id, week_id) WHERE kind = 'started';

CREATE TRIGGER rust_work_span_event_one_open
BEFORE INSERT ON rust_work_span_event
WHEN NEW.kind = 'started' AND EXISTS (
    SELECT 1 FROM (
        SELECT span_id FROM rust_work_span_event
        WHERE subject_id = NEW.subject_id AND kind = 'started'
        ORDER BY rowid DESC LIMIT 1
    ) AS newest
    WHERE NOT EXISTS (
        SELECT 1 FROM rust_work_span_event AS closed
        WHERE closed.subject_id = NEW.subject_id AND closed.span_id = newest.span_id AND closed.kind = 'closed'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'rust_work_span_event: another span is open; close it first');
END;

CREATE TRIGGER rust_work_span_event_closes_a_start
BEFORE INSERT ON rust_work_span_event
WHEN NEW.kind = 'closed' AND NOT EXISTS (
    SELECT 1 FROM rust_work_span_event
    WHERE subject_id = NEW.subject_id AND span_id = NEW.span_id AND kind = 'started'
)
BEGIN
    SELECT RAISE(ABORT, 'rust_work_span_event: a span closes only once it has started');
END;

CREATE TRIGGER rust_work_span_event_is_append_only
BEFORE UPDATE ON rust_work_span_event
BEGIN
    SELECT RAISE(ABORT, 'rust_work_span_event is append-only: a span is never rewritten');
END;

CREATE TRIGGER rust_work_span_event_is_kept
BEFORE DELETE ON rust_work_span_event
WHEN EXISTS (SELECT 1 FROM account WHERE subject_id = OLD.subject_id)
BEGIN
    SELECT RAISE(ABORT, 'rust_work_span_event is append-only: a span goes only with its account');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12).
CREATE TRIGGER rust_work_span_event_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM rust_work_span_event WHERE subject_id = OLD.subject_id;
END;

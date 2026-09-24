-- #286 (TEL1): one row per activity *block* a subject played, and how it went.
-- The history `EngagementClass::Momentum` ("finishing what you start") and
-- `Mastery` ("is it landing") are named for, and that #289's
-- `GET /subjects/me/stats` will read. Ingestion is #287 (TEL2); nothing writes
-- this table yet.
--
-- THE GRAIN: per activity block, not per session. A session is a tuple, and
-- the client's composer permits repeats on purpose -- `defaultSessionName`
-- labels them `Name xN`, and the catalogue's own comment calls two Honeycomb
-- blocks with different modes "a valid, expected session shape, not a
-- duplicate to collapse". One row per session would throw away *which block
-- lost them*, which is the whole distinction Momentum and Mastery are built on.
-- `StudySignal::ScoredBelowTarget { activity_id, score }` is already keyed by
-- activity, so the domain has assumed this grain since it was written.
--
-- WHAT THIS GRAIN IS NOT (#329, amendment 3). LeetType's rewritten surface
-- (`paulgsc/some-ui#1232`) grades a *ledger transition on an identified
-- proposition* -- a `CW-P` identifier entering `exposed` / `recognized` /
-- `demonstrated`. That is a different, finer grain again (the same way
-- item-level quiz detail is, which #286 already puts out of scope), and it does
-- not go in this table: a row here for a LeetType block records attendance --
-- whether the block was completed, abandoned, or skipped, and for how long --
-- and its `score` stays NULL unless the applet has a genuine assessment for the
-- block as a whole. Round completion on that surface is attendance by
-- construction (one session cannot satisfy the ledger's retention conjunct),
-- which is exactly the conflation #288 exists to end. Proposition transitions
-- get their own table beside #324's corpus, keyed by `CW-P` identifier.
--
-- Columns, each for a reason:
--
--   subject_id   -- #259; every table the policy reads is keyed by subject.
--   session_id   -- which session the block was part of. No REFERENCES: no
--                   table in this schema declares one, and an outcome is
--                   history -- it outlives a session the person later deletes,
--                   and is bounded by the retention rule below instead.
--   activity_id  -- which catalogue entry (#269's `activities.id`). Same
--                   no-REFERENCES reasoning: a catalogue entry can be retired
--                   without rewriting what someone did with it.
--   block_index  -- WHICH block, zero-based into the session's `activities`.
--                   Two Honeycomb blocks in one session need distinguishing,
--                   and this is the field that does it.
--   started_at, ended_at
--                -- ISO-8601 UTC. `ended_at` is the event timestamp the
--                   retention sweep reads. A skipped block has
--                   `started_at = ended_at` (the moment it was skipped) and
--                   `elapsed_ms = 0`, rather than a NULL `started_at` every
--                   reader would have to special-case.
--   planned_ms   -- what was scheduled for this block.
--   elapsed_ms   -- what actually happened.
--   outcome      -- completed | abandoned | skipped. Parsed strictly on the
--                   Rust side (`outcome_repo::OutcomeKind::parse`, refusing
--                   anything else the way `SessionStatus::parse` does) and
--                   CHECKed here too, so a bad value cannot be written by a
--                   path that forgot to parse.
--   score        -- REAL in [0, 1], only on a completed block (CHECKed), and
--                   NULLABLE ON PURPOSE. NULL means *not
--                   assessed*: `honeycomb`'s endless mode has no completion,
--                   an abandoned block has no assessment at all, and not every
--                   activity grades. It must never be conflated with 0.0,
--                   which means "assessed, and got nothing right". A
--                   `NOT NULL DEFAULT 0` would encode "abandoned" and "scored
--                   zero" as the same fact -- precisely the conflation #288
--                   exists to undo -- so every consumer has to handle NULL,
--                   and #289 excludes it from means rather than counting it.
--
-- THE IDEMPOTENCY KEY: UNIQUE (session_id, block_index). The client's signals
-- are fire-and-forget by design, and fire-and-forget plus retry means
-- duplicates. One block of one session yields one outcome, so a replayed
-- request is an upsert against this key rather than a second row -- and, per
-- #287, a replay must not fold a signal in twice either.
--
-- RETENTION: per #265's rule (docs/study-nudge.md, "History has a horizon"),
-- cited rather than re-derived -- time-based on the row's own event timestamp
-- (`ended_at`), an index on it, and a bounded sweep from the waker's pass. The
-- horizon is `outcome_repo::ACTIVITY_OUTCOME_RETENTION_DAYS`; its reasoning is
-- on the constant.
CREATE TABLE activity_outcome (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id   TEXT    NOT NULL,
    session_id   TEXT    NOT NULL,
    activity_id  TEXT    NOT NULL,
    block_index  INTEGER NOT NULL CHECK (block_index >= 0),
    started_at   TEXT    NOT NULL,   -- ISO-8601 UTC
    ended_at     TEXT    NOT NULL,   -- ISO-8601 UTC; the retention key
    planned_ms   INTEGER NOT NULL CHECK (planned_ms >= 0),
    elapsed_ms   INTEGER NOT NULL CHECK (elapsed_ms >= 0),
    outcome      TEXT    NOT NULL CHECK (outcome IN ('completed', 'abandoned', 'skipped')),
    -- Only a completed block can carry a score, and only in [0, 1]: an
    -- abandoned or skipped block is unassessed by definition, and a score on
    -- one would be averaged in by #289's stats as if it had been.
    score        REAL             CHECK (score IS NULL OR (outcome = 'completed' AND score >= 0.0 AND score <= 1.0)),

    UNIQUE (session_id, block_index)
);

-- "This subject's outcomes for this activity" -- the query #289 runs, newest
-- first for "last played at".
CREATE INDEX idx_activity_outcome_subject_activity ON activity_outcome(subject_id, activity_id, ended_at DESC);

-- The retention sweep's range read (#265's rule, point 2).
CREATE INDEX idx_activity_outcome_ended_at ON activity_outcome(ended_at);

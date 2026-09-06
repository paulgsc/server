-- #283 (RCM6): a session must be able to say whether a person composed it or
-- the waker proposed it, because `Momentum` reads the answer and gets it
-- backwards if it guesses. See `SessionOrigin::parse` (`session_repo::model`)
-- for the "unrecognised value is refused, not defaulted" argument -- the
-- same one already made for `SessionStatus::parse`.
--
-- `origin TEXT NOT NULL` cannot be added via `ALTER TABLE ... ADD COLUMN`
-- without a default, and a lingering default is exactly what the acceptance
-- criteria forbid -- the same footgun
-- `20260816000500_add_subject_to_sessions.up.sql` already reasoned through
-- for `subject_id`. So this rebuilds the table the same way that migration
-- did: a fresh `sessions_new` declares `origin` `NOT NULL` with no default,
-- an `INSERT ... SELECT` backfills every existing row, and the rebuilt table
-- is swapped in.
--
-- **The backfill is not a blind `'user'` literal.** `#282` (RCM5) shipped
-- before this migration and its waker has been provisioning real sessions
-- through `SessionRepository::provision_if_absent` since; a deployment that
-- has been running since then can already hold real system-provisioned rows
-- by the time this migration runs against it. Backfilling every row `'user'`
-- unconditionally would misclassify exactly those rows -- the ones this
-- story's abandonment guard exists to protect -- permanently, since nothing
-- downstream of this migration can ever recover which rows the waker wrote.
-- A real Codex review finding on this PR (`paulgsc/server#334`) caught this.
--
-- There is no column that says "the waker wrote this" -- `#272` already
-- decided `activities`/`scenes` share one JSON shape between the waker and a
-- person's own composer, so nothing in the payload distinguishes them. What
-- *is* reconstructable is whether a row still looks exactly like what
-- `materialize_provisioned_session` (`nudge::waker.rs`) leaves behind and
-- nothing has touched since: `status = 'scheduled'`, `scenes = '[]'`,
-- `layout IS NULL`, `layout_mode = 'basic'` -- every one of those is a
-- literal `materialize_provisioned_session` always writes, together.
--
-- **Those four alone are not enough** -- a second real Codex finding on this
-- PR caught that a proposal a person renamed, or whose `activities` they
-- edited, without ever touching status/scenes/layout/layout_mode, would
-- still match all four and stay `'system'` forever, contradicting PRO1's own
-- "editing makes it theirs" rule (`paulgsc/some-ui#1052`) this backfill is
-- supposed to honour retroactively. The fix adds a fifth condition:
-- `created_at = updated_at`. `materialize_provisioned_session` writes both
-- to the identical timestamp at creation, and `update_session`
-- unconditionally advances `updated_at` on *every* `PATCH` regardless of
-- which fields it actually touches -- so any edit at all, a rename included,
-- pulls `updated_at` away from `created_at`. Combined with the other four,
-- this is not just best-effort: `create_session` and `duplicate_session`
-- both hardcode `status: Draft` and never let a caller set `scheduled`
-- directly, so `materialize_provisioned_session` is the *only* write path in
-- this codebase that can produce a row with `status = 'scheduled'` at the
-- same instant as its own `created_at` -- any row matching all five was
-- provably written by the waker and never touched since, given every
-- server-side write path that exists today.
--
-- A row matching all five backfills `'system'`; every other row backfills
-- `'user'`, which is correct not just for a genuinely person-composed row
-- but *also* for a system-provisioned row that has since been started,
-- completed, renamed, or otherwise edited -- PRO1's own rule, applied
-- retroactively to whatever this backfill cannot otherwise see.
--
-- The one residual gap the four-condition version left open -- a person
-- creating an empty draft and `PATCH`ing its status straight to `scheduled`
-- without touching scenes or layout -- is closed by the fifth condition in
-- every practical case: `update_session` always computes `updated_at` from
-- a fresh `Utc::now()` call made strictly after the row's own `created_at`
-- was written, so the two would only read as equal strings on an exact
-- nanosecond-level clock collision across two separate requests, not a
-- realistic occurrence in any real deployment.
CREATE TABLE sessions_new (
    id                TEXT    PRIMARY KEY,
    subject_id        TEXT    NOT NULL,
    name              TEXT    NOT NULL,
    status            TEXT    NOT NULL,
    origin            TEXT    NOT NULL,      -- user | system; see SessionOrigin::parse
    layout_mode       TEXT    NOT NULL,
    total_duration_ms INTEGER NOT NULL,

    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    started_at        TEXT,
    completed_at      TEXT,
    final_elapsed_ms  INTEGER,

    activities        TEXT    NOT NULL,
    scenes            TEXT    NOT NULL,
    layout            TEXT
);

INSERT INTO sessions_new (
    id, subject_id, name, status, origin, layout_mode, total_duration_ms,
    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
    activities, scenes, layout
)
SELECT
    id, subject_id, name, status,
    CASE
        WHEN status = 'scheduled' AND scenes = '[]' AND layout IS NULL
         AND layout_mode = 'basic' AND created_at = updated_at
        THEN 'system'
        ELSE 'user'
    END,
    layout_mode, total_duration_ms,
    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
    activities, scenes, layout
FROM sessions;

DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;

-- Unchanged from `20260816000500_add_subject_to_sessions.up.sql` -- neither
-- index reasons about `origin`, and nothing reads sessions filtered or
-- ordered by it yet.
CREATE INDEX idx_sessions_started_at ON sessions(started_at);
CREATE INDEX idx_sessions_completed_at ON sessions(completed_at);
CREATE INDEX idx_sessions_status ON sessions(subject_id, status, updated_at DESC);

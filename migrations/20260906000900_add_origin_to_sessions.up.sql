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
-- literal `materialize_provisioned_session` always writes, together. A row
-- matching all four backfills `'system'`; every other row backfills
-- `'user'`, which is correct not just for a genuinely person-composed row
-- but *also* for a system-provisioned row that has since been started,
-- completed, or otherwise edited (any of those changes at least one of the
-- four columns) -- PRO1's own "editing makes it theirs" rule
-- (`paulgsc/some-ui#1052`), applied retroactively to whatever this backfill
-- cannot otherwise see.
--
-- This is a best-effort reconstruction, not a certain one: a person could in
-- principle create an empty draft and `PATCH` its status straight to
-- `scheduled` without ever touching scenes or layout, producing a row this
-- backfill cannot distinguish from a real proposal. That residual case is
-- accepted rather than hidden -- there is no signal left in this schema that
-- would resolve it, and it is narrower than the alternative of silently
-- misclassifying every already-provisioned row this deployment holds today.
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
        WHEN status = 'scheduled' AND scenes = '[]' AND layout IS NULL AND layout_mode = 'basic'
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

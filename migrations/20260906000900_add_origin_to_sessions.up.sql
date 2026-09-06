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
--
-- **A third real Codex finding on this PR: `status = 'scheduled'` alone
-- misses every row the waker wrote before `#282` (RCM5) existed.** RCM2
-- (`#313`) shipped the *first* version of this function, `provisioned_
-- session` (renamed to `materialize_provisioned_session` and rewritten by
-- RCM5 -- see `git show 9227d18:apps/servers/file_host/src/nudge/waker.rs`
-- for the exact original), and it wrote a materially different shape:
-- `status: Draft`, `name: "Suggested for you"`, `activities: []` (RCM3's
-- `recommend()` didn't exist yet to fill it in), plus the same `scenes: []`,
-- `layout: None`, `layout_mode: Basic`, `total_duration_ms: 0`, and
-- `created_at == updated_at` the current version still writes. A deployment
-- that has been running since RCM2 landed can hold real proposals in this
-- exact legacy shape, untouched, and `status = 'scheduled'` alone silently
-- excludes every one of them.
--
-- Simply widening `status = 'scheduled'` to `status IN ('scheduled',
-- 'draft')` is wrong on its own -- `'draft'` is also `create_session`'s own
-- initial status for a real person's fresh, still-empty session (the
-- `session-user-empty-draft` case this migration's own tests already cover),
-- and that row shares every other column value with the legacy waker shape
-- too (`scenes: []`, `layout: NULL`, `layout_mode: 'basic'`, `created_at ==
-- updated_at`). The one column that actually distinguishes them is `name`:
-- `"Suggested for you"` is a literal only the legacy waker code ever wrote,
-- and `create_session`'s `name` is a required, client-supplied field with no
-- default, so a real person's draft coinciding with that exact string is not
-- a case this backfill needs to guard against. `activities = '[]'` is
-- checked alongside it for the same reason: RCM2's own version always wrote
-- an empty `activities`, which a real composed session with that literal
-- name would be an even less likely coincidence to also match.
--
-- So this is two disjoint cases, matching each waker version's own exact
-- output: the current (post-RCM5) shape via `status = 'scheduled'` plus the
-- original four conditions, or the legacy (RCM2-RCM4) shape via `status =
-- 'draft' AND name = 'Suggested for you' AND activities = '[]'` plus the
-- same shared conditions (`scenes = '[]'`, `layout IS NULL`, `layout_mode =
-- 'basic'`, `created_at = updated_at`) both versions always wrote.
--
-- **A fourth real Codex finding on this PR: requiring `status = 'scheduled'`
-- (or `'draft'`) also excludes a provisioned session that was started,
-- paused, or completed but never actually edited.** `session_abandonment_
-- is_real`'s own design (`session_repo::model`) is explicit that a
-- `system`-origin session which *was* opened stays `system` -- the
-- abandonment check reads `started_at` directly, not a forced origin flip;
-- "starting it is a real action, even if renaming it is not." A provisioned
-- row a person merely pressed Start (or Pause, or Complete) on, without
-- editing its content, should backfill exactly the same as an untouched one
-- -- but a lifecycle transition changes `status` away from `scheduled`/
-- `draft` and bumps `updated_at` identically to a real edit, so the
-- conditions above alone misclassified it as `user`.
--
-- The two clauses above catch "definitely untouched." A third clause below
-- catches "possibly moved through its lifecycle, but nothing here proves a
-- *content* edit happened too": `status IN ('active', 'paused', 'completed')`
-- or any of `started_at`/`completed_at`/`final_elapsed_ms` populated --
-- every one of those is state only a lifecycle transition produces, and
-- none of them is reachable by renaming or editing activities alone. This
-- clause deliberately does not require `created_at = updated_at` (a
-- lifecycle transition bumps it exactly like an edit would) or the fixed
-- name/activities check (RCM5's own naming is dynamic, not the literal
-- `"Suggested for you"` only the legacy shape has).
--
-- **The honest residual, one layer narrower than before:** this cannot
-- distinguish "started, never edited" from "started, *and* activities were
-- edited in a separate request" -- both leave the same lifecycle evidence,
-- and there is no stored baseline of the waker's original `activities` to
-- compare against. Editing an already-started proposal's activities is a
-- narrower case than the plain rename/edit-while-still-scheduled case the
-- first two clauses already catch correctly, and it is accepted for the
-- same reason as every residual case named above: this backfill runs once,
-- from column values alone, with no audit log of what specifically changed
-- and why -- and the story's own stated bias is to protect against a
-- proposal reading as abandoned when nobody engaged with it, which this
-- clause exists for, not the rarer reverse.
--
-- The "empty draft manually pushed through `PATCH`" case named above is
-- *not* a residual here, even though the lifecycle clauses accept any
-- status: `activities != '[]'` (post-RCM5) and `name = 'Suggested for
-- you'` (legacy) are what gate each one, and a real empty user draft
-- matches neither -- `activities: []` fails the first, and a real person's
-- required, client-supplied `name` coinciding with the legacy literal
-- fails the second, the same coincidence already ruled out above. A real
-- Codex review finding on this PR caught that the first version of this
-- clause checked lifecycle evidence alone, which an empty user draft
-- pushed straight to `active`/`paused`/`completed` could satisfy too --
-- fixed by requiring one of these two content-based gates alongside it.
--
-- **`activities != '[]'` proves necessity, not sufficiency, and that is the
-- honest remaining residual.** `an_empty_provisioned_session_is_never_
-- persisted...` proves every real waker row has non-empty `activities` --
-- it does not prove the converse, that non-empty `activities` implies the
-- waker wrote it. A real person's own Basic session, composed with real
-- activities through the normal client flow, is assumed to reach the
-- server with `scenes` already populated too (`session-composer.tsx` reads
-- `existingSession.scenes` only for the `advanced` editor, implying the
-- Basic composer computes and sends real scenes at save time, the same
-- normal-flow assumption every other case in this migration's comments
-- already leans on) -- so it would fail this backfill's outer `scenes =
-- '[]'` requirement regardless. The only way to reach `activities != '[]'`
-- *and* `scenes = '[]'` as a real user session is to bypass that normal
-- composer flow entirely and hand-craft the request directly -- the same
-- precondition every other residual named in this file already requires,
-- not a new category of risk this fix introduces.
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
        WHEN scenes = '[]' AND layout IS NULL AND layout_mode = 'basic'
         AND (
             -- Still exactly as either waker version left it: nothing has
             -- touched the row at all since creation.
             (created_at = updated_at AND status = 'scheduled')
             OR (created_at = updated_at AND status = 'draft' AND name = 'Suggested for you' AND activities = '[]')
             -- Or it has moved through a lifecycle transition (Start/Pause/
             -- Complete) since -- evidence of that is checkable directly
             -- (status left 'scheduled'/'draft', or any of the three
             -- lifecycle timestamps populated), and per
             -- `session_abandonment_is_real`'s own design a started `system`
             -- session must *stay* `system` -- the abandonment check reads
             -- `started_at`, not a forced origin flip. This branch does not
             -- require `created_at = updated_at`, since starting a session
             -- bumps `updated_at` exactly like an edit would and the two
             -- are not otherwise distinguishable from this row alone.
             --
             -- Lifecycle evidence alone is not enough, though: an empty
             -- real user draft (`create_session`'s own shape, activities:
             -- []) can reach `active`/`paused`/`completed` too, by a direct
             -- `PATCH`, without ever being edited into something non-empty
             -- -- a real Codex finding on this PR. `activities != '[]'` is
             -- what actually rules that out for the post-RCM5 shape,
             -- provably rather than heuristically: `consider` (`nudge::
             -- waker.rs`) checks `provisioned.activities.is_empty()` before
             -- ever calling `provision_if_absent`, so a real waker-written
             -- row can never have empty `activities` in the first place
             -- (the `an_empty_provisioned_session_is_never_persisted...`
             -- test pins exactly this). The legacy shape is the mirror
             -- image -- its `activities` is *always* empty, so it is
             -- matched by `name = 'Suggested for you'` instead, the same
             -- literal-string signal the untouched-legacy-row branch above
             -- already relies on.
             OR (
                 activities != '[]'
                 AND (status IN ('active', 'paused', 'completed') OR started_at IS NOT NULL OR completed_at IS NOT NULL OR final_elapsed_ms IS NOT NULL)
             )
             OR (
                 name = 'Suggested for you'
                 AND (status IN ('active', 'paused', 'completed') OR started_at IS NOT NULL OR completed_at IS NOT NULL OR final_elapsed_ms IS NOT NULL)
             )
         )
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

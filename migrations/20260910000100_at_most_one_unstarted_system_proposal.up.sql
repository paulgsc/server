-- #284 (RCM7): a persistently disengaged subject becomes eligible again
-- roughly daily (REFRACTORY = 20h, recharge_on_intervention gives Presence
-- back 30/nudge) -- without a guard, a fortnight of ignored notifications
-- would produce fourteen stacked proposals.
--
-- The rule, quoted from the issue directly: before provisioning, if the
-- subject already has an `origin = 'system' AND started_at IS NULL` session,
-- do not create another. `origin` alone (#283/RCM6) is not enough --
-- `started_at IS NULL` is what excludes a `system` session the person did
-- start: that one is history now, not a proposal, and this rule must never
-- touch it (`session_abandonment_is_real`'s own design already reads
-- `started_at` for the identical reason). A `system` session promoted to
-- `user` by editing (RCM6's one-way `upsert` guard) falls out of this rule
-- automatically too, for free, once the predicate is `origin = 'system'`
-- rather than merely "something is prepared".
--
-- **`status IN ('paused', 'scheduled', 'draft')` is a third, load-bearing
-- condition, not a redundant restatement of `started_at IS NULL` -- a real
-- `chatgpt-codex-connector` finding on `paulgsc/server#335` caught its
-- absence.** `SessionRepository::set_status_many` (`PATCH /sessions/status`)
-- writes only `status` and `updated_at` -- it can move a `system`-origin,
-- never-started proposal straight to `active` or `completed` while leaving
-- `started_at` `NULL`, a real, client-reachable path (`updateStatusMany` in
-- `apps/www/src/lib/tenant/sessions-repository.ts`, `paulgsc/some-ui`).
-- Without this clause, such a row still matches `origin = 'system' AND
-- started_at IS NULL` even though `first_prepared` (whose own tracked set is
-- exactly `paused`/`scheduled`/`draft`) can never find it again -- it
-- permanently occupies this index's one slot, starving the subject of every
-- future proposal, and remains a live `ON CONFLICT` target
-- `SessionRepository::provision_or_refresh` would silently overwrite the
-- content of on the next `NothingToSay` pass. Restricting the predicate to
-- the same three statuses `first_prepared` already tracks closes both: such
-- a row falls out of the index (a fresh proposal can be provisioned
-- alongside it) and out of `provision_or_refresh`'s conflict target (nothing
-- ever overwrites it).
--
-- Enforced as a real constraint, not just application-level care: a partial
-- unique index over exactly the rows this rule cares about. Unlike
-- `idx_sessions_status` (`(subject_id, status, updated_at DESC)`, #263/SLI2's
-- own three-probe bounded read), this index is scoped to the predicate the
-- rule actually is -- a subject can have any number of `completed`/`paused`/
-- `user`-origin rows, and now any number of `active`/`completed` `system`
-- rows with `started_at IS NULL` too (the `set_status_many` anomaly above);
-- the constraint only ever looks at the ones matching all three conditions.
-- That scoping is also what makes `SessionRepository::provision_or_refresh`'s
-- single `INSERT ... ON CONFLICT (subject_id) WHERE ...` statement possible:
-- SQLite requires an `ON CONFLICT` target's `WHERE` clause to match a real
-- partial index verbatim, and this is that index.
--
-- Bounded per #253: the write path below is one indexed statement (an
-- equality probe against this index), not a scan -- the same "a read/write
-- reachable from the waker declares its own bound" discipline
-- `first_prepared`'s three single-status probes already established for the
-- neighbouring query.
--
-- Defensive cleanup first, the same caution `20260906000900_add_origin_to_
-- sessions.up.sql` used for its own backfill: this migration can run against
-- a live deployment that has been provisioning sessions since `#279` (RCM2),
-- and `CREATE UNIQUE INDEX` fails outright if any subject already violates
-- it. **Scoped to the same three statuses the index itself now is** -- a
-- second real finding on this PR caught that the original, broader
-- `origin = 'system' AND started_at IS NULL` predicate would delete a real
-- `completed` (or `active`) session reached through the same
-- `set_status_many` anomaly: such a row is genuine history, not an ignored
-- proposal, and "none of these rows were ever started or opened" is only
-- true of the narrower set this migration now actually touches. Ties within
-- that narrower set are broken by `updated_at`, then `rowid`, favouring the
-- most recently touched candidate as the survivor.
DELETE FROM sessions
WHERE origin = 'system' AND started_at IS NULL AND status IN ('paused', 'scheduled', 'draft')
  AND rowid NOT IN (
      SELECT s2.rowid
      FROM sessions s2
      WHERE s2.subject_id = sessions.subject_id
        AND s2.origin = 'system' AND s2.started_at IS NULL AND s2.status IN ('paused', 'scheduled', 'draft')
      ORDER BY s2.updated_at DESC, s2.rowid DESC
      LIMIT 1
  );

CREATE UNIQUE INDEX idx_sessions_one_unstarted_system_proposal
ON sessions (subject_id)
WHERE origin = 'system' AND started_at IS NULL AND status IN ('paused', 'scheduled', 'draft');

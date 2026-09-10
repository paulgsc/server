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
-- Enforced as a real constraint, not just application-level care: a partial
-- unique index over exactly the rows this rule cares about. Unlike
-- `idx_sessions_status` (`(subject_id, status, updated_at DESC)`, #263/SLI2's
-- own three-probe bounded read), this index is scoped to the two-column
-- predicate the rule actually is -- a subject can have any number of
-- `completed`/`paused`/`user`-origin rows; the constraint only ever looks at
-- the ones matching `origin = 'system' AND started_at IS NULL`. That scoping
-- is also what makes `SessionRepository::provision_or_refresh`'s single
-- `INSERT ... ON CONFLICT (subject_id) WHERE ...` statement possible: SQLite
-- requires an `ON CONFLICT` target's `WHERE` clause to match a real partial
-- index verbatim, and this is that index.
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
-- it. In every deployment running the current `provision_if_absent` guard
-- (`WHERE NOT EXISTS (... status IN ('paused','scheduled','draft'))`) this is
-- unreachable -- that guard already refuses a second draft/scheduled/paused
-- row of *any* origin per subject, and `origin = 'system' AND started_at IS
-- NULL` rows are a subset of that -- but the index creation should not be
-- the thing that discovers a violation of an invariant this migration is
-- introducing, not merely documenting. Ties are broken by `updated_at`, then
-- `rowid`, favouring the most recently touched candidate as the survivor --
-- none of the deleted rows were ever started or opened, by construction of
-- the predicate they matched, so nothing a person actually did is lost.
DELETE FROM sessions
WHERE origin = 'system' AND started_at IS NULL
  AND rowid NOT IN (
      SELECT s2.rowid
      FROM sessions s2
      WHERE s2.subject_id = sessions.subject_id
        AND s2.origin = 'system' AND s2.started_at IS NULL
      ORDER BY s2.updated_at DESC, s2.rowid DESC
      LIMIT 1
  );

CREATE UNIQUE INDEX idx_sessions_one_unstarted_system_proposal
ON sessions (subject_id)
WHERE origin = 'system' AND started_at IS NULL;

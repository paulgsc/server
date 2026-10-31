# The schema: how each table lives

S13 (#511), under `[EPIC][DRIFT]` (#481). The schema's rules used to live in migration
comments and per-crate tests: hand-written append-only triggers (15 at the recon, 27 now), each
held only by its own crate's test where one exists, and a "No REFERENCES, matching every other table in this schema" repeated
in four migrations. This doc writes them down once, and
`apps/servers/file_host/src/lifecycle.rs` holds them against the migrated schema and the
source.

| Where | What |
|---|---|
| `apps/servers/file_host/src/lifecycle.rs` | `TABLE_LIFECYCLE`, `DECLARED_REFERENCES`, `APPEND_ONLY_REWRITES`, and the tests that hold this doc's invariants. |
| `apps/servers/file_host/src/subject.rs`, `privacy.rs` | Whose each table is (`SUBJECT_SCOPED_TABLES`, `NOT_SUBJECT_SCOPED`): docs/identity.md. |
| paulgsc/some-ui `packages/assurance/src/ledger.json`, `stores` | What each table is worth, how it grows and when it is let go; `scripts/check_assurance_ledger.py` fails a table no store lists (docs/assurance.md). |
| paulgsc/some-ui `docs/discard-sites.md`, "On home" | Every place home deletes or rewrites what it keeps, with the owner's ruling where one exists (#2265 there). |

Three lists, three questions: whose a table is (identity), what it is worth (the ledger), and
whether its rows can change once written (this doc). They do not repeat each other. In
particular the ledger's `class: record` means the owner's record, which is worth keeping; it is
not the same as append-only here. `sessions` and `learner_shelf` are records in the ledger and
mutable here, and that overlap is exactly what `docs/discard-sites.md` lists. `voice_transcript`
is `derived` in the ledger and append-only here.

## Lifecycles

Every table the migrations leave standing has exactly one. Each describes the code as it is,
not as it might become.

| Lifecycle | Means | Tables (at `8063e1a`) |
|---|---|---|
| `AppendOnly` | Written once, never updated: a trigger aborts every `UPDATE`. A row leaves only as its triggers allow: with its account (`<t>_leaves_with_account`, docs/identity.md invariant 12), or never (`<t>_is_kept`, `<t>_no_delete`). #511 calls these "records"; the name here says the mechanism, so it does not collide with the ledger's `record`. | 27 |
| `State` | Current state: a row is rewritten or deleted in place, and the value before is not kept. | 9 (with sqlx's `_sqlx_migrations`) |
| `Corpus` | Content whose source lives outside this database (a seed migration, git, some-ui's corpus), and what is derived from it. | 6 |
| `Ephemeral` | Rows that leave by design after a horizon or beyond a cap, not by anyone's act. | 4 |
| `Legacy` | Kept from an earlier era of the server. Whether it stays is S12's call 2 (#492), not this doc's. | 2 (`mood_events`, `tabs`) |

`TABLE_LIFECYCLE` gives each table's reason. Three of them name an owner call pending on S12
(#492): `activity_outcome` and `intervention_log` (call 1, the sweeps), and `activities` (call 5,
the seed). They are classified by today's code, and the call may move them.

## Invariants

1. **Every table has a lifecycle.** *Enforced by*
   `every_table_has_a_lifecycle_and_every_append_only_table_is_held_by_its_trigger`, which reads
   `sqlite_master` after `MIGRATOR` runs: a new table fails until it is in `TABLE_LIFECYCLE`, and
   a listed table no migration creates fails too.
2. **An append-only table is append-only by construction.** It carries a trigger that aborts
   every `UPDATE`: `BEFORE UPDATE ON <t>`, no `WHEN`, and a body that is only
   `SELECT RAISE(ABORT, '...')`. The name does not matter (`<t>_is_append_only`, or
   `<t>_no_update` for the game feed and `storage_sample`). *Enforced by* the same test, both
   ways: an append-only table without such a trigger fails, and so does a table with one that is
   classified as anything else. So a table rebuild that forgets to recreate the trigger fails here,
   whatever that table's own crate tests (`a_table_rebuilt_without_its_trigger_fails_the_lifecycle_check`).
   S14 (#512) decides whether a new `CHECK` value goes on costing such a rebuild. This invariant
   holds either way. *It does not cover* what deletes a trigger allows: most append-only tables let
   their rows go with the account, by design.
3. **No production SQL rewrites an append-only table.** *Enforced by*
   `no_production_sql_rewrites_an_append_only_table`. It reads the string literals of every
   non-test `.rs` file under `crates/db` and `apps/servers/file_host/src`, and fails on an
   `UPDATE`, a `DELETE FROM`, a `REPLACE INTO` (or `INSERT OR REPLACE`), or an upsert's
   `DO UPDATE` against an append-only table. The only way through is an entry in
   `APPEND_ONLY_REWRITES` that says why that row may change, the way a `Lifetime:` line does in
   paulgsc/some-ui. An entry that matches nothing fails too. The list is empty: no such site
   exists. Test code is read out lexically: `#[cfg(test)]`, `#[test]`, `#[tokio::test]` and
   `#[sqlx::test]` items, out-of-line test modules, `#![cfg(test)]` files, and `tests/`,
   `benches/` and `examples/` directories. *It does not see* SQL built at run time. Account
   deletion's sweep over `SUBJECT_SCOPED_TABLES` (`handlers::auth::delete_account`) is the one
   such delete of append-only rows. It is the owner's own act (docs/identity.md invariant 9), and
   the trigger is what holds everything else.
4. **No reference touches a subject-scoped table.** History outlives the row it points to, and
   deletion goes by subject, table by table: account deletion's sweep, plus a
   `<t>_leaves_with_account` trigger for every table newer than it (docs/identity.md invariants 9
   and 12). A `REFERENCES` to or from such a table would either block that walk or delete by a
   path the walk does not name. So a subject-scoped table names another table by value only.
   *Enforced by* `every_declared_reference_is_listed_and_touches_no_subject`.
5. **A declared reference is listed and cannot block a delete.** `REFERENCES` appears only
   between tables that name no subject. Its parent is either append-only and never deleted, so the
   reference only guards insertion, or the child is derived from the parent and goes with it
   (`ON DELETE CASCADE`). Every one is in `DECLARED_REFERENCES` with why, and the same test fails
   on one that is not listed, one that is listed but gone, or one that breaks either rule. #511
   found the LeeType pair undocumented. There are seven now:

   | Child | Parent | Why |
   |---|---|---|
   | `leetype_round_run.round_id`, `leetype_round_witness.round_id` | `leetype_round` | Derived from the round's body, rewritten with it, and gone with it (`CASCADE`). |
   | `game_feed_response.sha256` | `game_feed_body` | A fetch names its body by hash. A body is never deleted (STORAGE4, #555). |
   | `game_feed_play.response_id`, `game_feed_play.supersedes` | `game_feed_response`, `game_feed_play` | A play version names its fetch and the version it revises. None is ever deleted. |
   | `rust_round_run.source_seq`, `rust_round_drop.source_seq` | `rust_round_source` | A run or drop names its program. A program is never deleted (`rust_round_source_is_kept`). |

6. **Implied references are kept by deletion order, not by the database.** Nothing checks these.
   Each is a column that names another table's row by value:

   - `activity_outcome.session_id` → `sessions`;
   - `activity_outcome.activity_id`, `curriculum.activity_id` and `learner_shelf.activity_id` →
     `activities`;
   - `curriculum_publication.curriculum_id` → `activities` or `curriculum`, depending on `source`;
   - `voice_note_audio`, `voice_transcript`, `voice_transcript_word`, `mailbox_transcript` and
     `game_bite`'s `note_id` → `voice_note`;
   - `mailbox_leetype_log.session_id` → `leetype_session_event.session_id`;
   - `studio_friction.sitting_id` → `studio_sitting_event.sitting_id`.

   Where both ends are subject-scoped, they leave together with the account. Of the parents
   above, only two can go while their children stay. A `sessions` row the owner deletes leaves
   its outcomes behind, on purpose: the migration says outcomes outlive their session. An
   `activities` row would need a seed migration to retire it. Every other parent is append-only
   and has no production delete. A reader that joins one of these must treat a missing parent as
   normal.

## Discard sites

This doc keeps no list of its own. paulgsc/some-ui's `docs/discard-sites.md`, "On home", names
every site where home deletes or rewrites what it keeps, with the owner's ruling where one
exists (paulgsc/some-ui#2265 holds the rest). The two sweeps there are S12's call 1 (#492):
`activity_outcome` after 365 days, whose horizon the owner kept (#541), and `intervention_log`
after 90 days. Both run only while the nudge is on (#556). Invariant 3 is what this repository
adds to that list: an append-only table has no discard site at all, except deletion with its
account.

At `8063e1a`, every rewrite the invariant 3 scan finds on a table that is not append-only is in
that list. Each of these is a site there: `sessions`' delete, `delete_many`, upsert and refresh;
`learner_shelf`'s overwrite and delete; the two sweeps; and the expected deletes in
`push_subscriptions`, auth and presence. Two `UPDATE`s fill a column and discard nothing:
`intervention_log.actuated_at` and `auth_session.expires_at`'s slide.

## Adding a table

A new table needs all of these in the PR that creates it. Each fails a check until it is there:

- a `server:<table>` store in some-ui's ledger, landed first (`Assurance ledger`, docs/assurance.md);
- `SUBJECT_SCOPED_TABLES` or `NOT_SUBJECT_SCOPED`, and, if subject-scoped, its kind in
  docs/identity.md's "What deleting an account erases" and an `AFTER DELETE ON account` trigger
  (`privacy.rs`);
- a lifecycle in `TABLE_LIFECYCLE`, and if it is `AppendOnly`, its trigger (`lifecycle.rs`).

A rebuild of an existing table (to widen a `CHECK`, say) recreates every trigger it dropped,
both `<t>_is_append_only` and `<t>_leaves_with_account`. Each rule above fails one it forgets.

## Baselines (re-measured at `8063e1a`, 2026-10-31)

| Measure | At the recon (`4acfe90`) | Now |
|---|---|---|
| Tables | 35 | 48, with `_sqlx_migrations` |
| Append-only tables, held by a trigger | 15 | 27 (23 `<t>_is_append_only`, 4 `<t>_no_update`) |
| `AFTER DELETE ON account` triggers | 15 | 20 |
| Tables declaring `REFERENCES` | 2 | 6 (7 references) |
| Discard sites on home | 6 (named in #511) | 28 rows in some-ui's `docs/discard-sites.md`, account deletion included |
| Production rewrites of an append-only table | not measured | 0 |

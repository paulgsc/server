# The schema: how each table lives

S13 (#511), under `[EPIC][DRIFT]` (#481). The schema's rules used to live in migration
comments and per-crate tests: hand-written append-only triggers (15 at the recon, 74 now), each
held only by its own crate's test where one exists, and a "No REFERENCES, matching every other
table in this schema" repeated in four migrations. This doc writes them down once, and
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
mutable here, and that overlap is exactly what `docs/discard-sites.md` lists.

## Lifecycles

Every table the migrations leave standing has exactly one. Each describes the code as it is,
not as it might become, and none is a ruling on whether it should stay that way.

| Lifecycle | Means | Tables (at `14eb6c2`) |
|---|---|---|
| `AppendOnly` | Written once, never updated: a trigger aborts every `UPDATE`. A row leaves only as its triggers allow: with its account (`<t>_leaves_with_account`, docs/identity.md invariant 12), never (`<t>_is_kept`, `<t>_no_delete`), or at a discard site `APPEND_ONLY_REWRITES` names. #511 calls these "records"; the name here says the mechanism, so it does not collide with the ledger's `record`. | 74 |
| `State` | Current state: a row is rewritten or deleted in place, and the value before is not kept. A week's draft (`rust_week_draft`, `interview_week_draft`) is state; its final is append-only. | 11 (with sqlx's `_sqlx_migrations`) |
| `Corpus` | Content whose source lives outside this database (a seed migration, git, some-ui's corpus), and what is derived from it (`voice_note_meaning`, made from a take's words). | 7 |
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
   `<t>_no_update`). *Enforced by* the same test, both ways: an append-only table without such a
   trigger fails, and so does a table with one that is classified as anything else. So a table
   rebuild that forgets to recreate the trigger fails here, whatever that table's own crate tests
   (`a_table_rebuilt_without_its_trigger_fails_the_lifecycle_check`). S14 (#512) decides whether
   a new `CHECK` value goes on costing such a rebuild. This invariant holds either way. *It does
   not cover* what deletes a trigger allows: most append-only tables let their rows go with the
   account, by design.
3. **No production SQL rewrites an append-only table, except at a listed discard site.**
   *Enforced by* `no_production_sql_rewrites_an_append_only_table`. It reads the string literals
   of every non-test `.rs` file under `crates/db` and `apps/servers/file_host/src`, and fails on
   an `UPDATE`, a `DELETE FROM`, a `REPLACE INTO` (or `INSERT OR REPLACE`), or an upsert's
   `DO UPDATE` against an append-only table. The only way through is an entry in
   `APPEND_ONLY_REWRITES` that says why that row may change, the way a `Lifetime:` line does in
   paulgsc/some-ui, and an entry that matches nothing fails too. Six entries stand, all in
   `crates/db/voice/src/delete.rs`: My data's delete of a take (MD3) removes its `voice_note`,
   audio, transcripts, words, set-aside mark and conveyor moves, and `let_go_audio` drops an
   interview retake's audio (IW8). Each is a site in some-ui's `docs/discard-sites.md` with the
   owner's ruling, and each leaves a receipt in `voice_note_deletion`. Test code is read out
   lexically: `#[cfg(test)]`, `#[test]`, `#[tokio::test]` and `#[sqlx::test]` items, out-of-line
   test modules, `#![cfg(test)]` files, and `tests/`, `benches/` and `examples/` directories.
   *It does not see* SQL built at run time. Account deletion's sweep over
   `SUBJECT_SCOPED_TABLES` (`handlers::auth::delete_account`) is the one such delete in Rust, and
   the migrations' `<t>_leaves_with_account` triggers do the same in SQL. Both are the owner's own
   act (docs/identity.md invariant 9), and the update trigger is what holds everything else.
   Each `APPEND_ONLY_REWRITES` entry also says how many such statements its file holds, so a
   new one beside a listed site fails too.
4. **No reference points at a subject-scoped table.** History outlives the row it points to, and
   deletion goes by subject, table by table: account deletion's sweep, plus a
   `<t>_leaves_with_account` trigger for every table newer than it (docs/identity.md invariants 9
   and 12). A `REFERENCES` to such a table would block that walk, or delete by a path the walk
   does not name. A subject-scoped *child* may name a parent that is never deleted: its own row
   still leaves with its account, and nothing waits on it. `cards_essay_video_deletion` is the
   one such child (WALK7). *Enforced by*
   `every_declared_reference_is_listed_and_blocks_no_subjects_deletion`.
5. **A declared reference is listed and cannot block a delete.** Its parent is either
   append-only and never deleted (a trigger aborts every `DELETE` of it, which the test checks),
   so the reference only guards insertion, or the child is derived
   from the parent and goes with it (`ON DELETE CASCADE`). Every one is in `DECLARED_REFERENCES`
   with why, a composite key as its columns in key order, and the same test fails on one that is
   not listed, one that is listed but gone, or one that breaks either rule. #511 found the LeeType
   pair undocumented. There are 18 now, on 15 tables:

   | Child | Parent | Why |
   |---|---|---|
   | `leetype_round_run.round_id`, `leetype_round_witness.round_id` | `leetype_round` | Derived from the round's body, rewritten with it, and gone with it (`CASCADE`). |
   | `game_feed_response.sha256` | `game_feed_body` | A fetch names its body by hash. A body is never deleted (STORAGE4, #555). |
   | `game_feed_play.response_id`, `game_feed_play.supersedes` | `game_feed_response`, `game_feed_play` | A play version names its fetch and the version it revises. None is ever deleted. |
   | `rust_round_run.source_seq`, `rust_round_drop.source_seq` | `rust_round_source` | A run or drop names its program. A program is never deleted (`rust_round_source_is_kept`). |
   | `call_week_revision`, `call_video_link`, `call_video_deletion`, `call_part_link`, `call_part_deletion` (`week_from`) | `call_week` | Each names the week it is about. A kept week is never deleted. |
   | `call_video_deletion(week_from, youtube_id)`, `call_part_deletion(week_from, part, youtube_id)` | `call_video_link`, `call_part_link` | A receipt names the link it let go. A link is never deleted. |
   | `cards_essay_link.essay_key`, `cards_essay_video_deletion.essay_key` | `cards_essay` | Each names its essay. A kept essay is never deleted. |
   | `pd_video_link(season, team, week)`, `pd_video_deletion(season, team, week, youtube_id)` | `pd_fill`, `pd_video_link` | A link names its fill, a receipt its link. Neither is ever deleted. |

6. **Implied references are kept by deletion order, not by the database.** Nothing checks these.
   Each is a column that names another table's row by value. The list is the recon's and the
   voice note's, not every one the schema holds:

   - `activity_outcome.session_id` → `sessions`;
   - `activity_outcome.activity_id`, `curriculum.activity_id` and `learner_shelf.activity_id` →
     `activities`;
   - `curriculum_publication.curriculum_id` → `activities` or `curriculum`, depending on `source`;
   - `note_id` → `voice_note`, from `voice_note_audio`, `voice_transcript`,
     `voice_transcript_word`, `voice_transcript_verdict`, `voice_note_meaning`,
     `voice_note_set_aside`, `voice_note_attention`, `studio_take_move`, `mailbox_transcript` and
     `game_bite`;
   - `mailbox_leetype_log.session_id` → `leetype_session_event.session_id`;
   - `studio_friction.sitting_id` → `studio_sitting_event.sitting_id`.

   A `sessions` row the owner deletes leaves its outcomes behind, on purpose: the migration says
   outcomes outlive their session. An `activities` row would need a seed migration to retire it.
   A `voice_note` goes only with its account or at My data's delete, which removes the take's own rows in the same
   transaction and leaves a receipt (`voice_repo::delete`); a row elsewhere that names it may stay. A reader that joins one of these must
   treat a missing parent as normal.

## Discard sites

This doc keeps no list of its own. paulgsc/some-ui's `docs/discard-sites.md`, "On home", names
every site where home deletes or rewrites what it keeps, with the owner's ruling where one
exists (paulgsc/some-ui#2265 holds the rest). The two sweeps there are S12's call 1 (#492):
`activity_outcome` after 365 days, whose horizon the owner kept (#541), and `intervention_log`
after 90 days. Both run only while the nudge is on (#556). Invariant 3 is what this repository
adds to that list: an append-only table has no discard site except deletion with its account
and the sites `APPEND_ONLY_REWRITES` names, and a new one fails until it is named.

## Adding a table

A new table needs all of these in the PR that creates it. Each fails a check until it is there:

- a `server:<table>` store in some-ui's ledger, landed first (`Assurance ledger`, docs/assurance.md);
- `SUBJECT_SCOPED_TABLES` or `NOT_SUBJECT_SCOPED`, and, if subject-scoped, its kind in
  docs/identity.md's "What deleting an account erases" and an `AFTER DELETE ON account` trigger
  (`privacy.rs`);
- a lifecycle in `TABLE_LIFECYCLE`, and if it is `AppendOnly`, its trigger (`lifecycle.rs`);
- any `REFERENCES` in `DECLARED_REFERENCES`, never to a subject-scoped table (`lifecycle.rs`).

A rebuild of an existing table (to widen a `CHECK`, say) recreates every trigger it dropped,
both `<t>_is_append_only` and `<t>_leaves_with_account`. Each rule above fails one it forgets.

## Baselines (re-measured at `14eb6c2`, 2026-11-21)

| Measure | At the recon (`4acfe90`, 10-26) | Now |
|---|---|---|
| Tables | 35 | 98, with `_sqlx_migrations` |
| Append-only tables, held by a trigger | 15 | 74 (58 `<t>_is_append_only`, 16 `<t>_no_update`) |
| `AFTER DELETE ON account` triggers | 15 | 49 |
| Tables declaring `REFERENCES` | 2 | 15 (18 references) |
| Discard sites on home | 6 (named in #511) | 32 rows in some-ui's `docs/discard-sites.md`, "On home" |
| Production rewrites of an append-only table | not measured | 6 tables, all at My data's delete and the interview retake let-go, each listed |

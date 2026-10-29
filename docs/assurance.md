# The assurance ledger, from this side

paulgsc/some-ui keeps one record of what the system promises about outcomes and work: the
assurance ledger (`packages/assurance/src/ledger.json` there; its grammar is some-ui's
`docs/operational-assurance.md`, its epic some-ui#2199). Each promise (an *objective*) has a
verdict, and one that `holds` names the proof that holds it, as `<repo>:<file>#<name>`.
some-ui's own check holds everything it can see, which stops at this repository. Two jobs
here hold the rest (#526):

- **`Assurance ledger`** (`.github/workflows/assurance.yml`, `scripts/check_assurance_ledger.py`):
  what the ledger says about this repository is still true here (objective GC4).
- **`PR gate`** (`.github/workflows/gate.yml`, `scripts/pr_gate.py`): one check that passes only
  when every other check a pull request runs here passed, for `main`'s ruleset to require
  (objective GC5).

Each script's docstring is the precise rule; this page says why, and where each one stops.

## `Assurance ledger`

It checks out some-ui's `main` the way `routes.yml` does (public, no token), finds the ledger by
its `schema` among the files named `ledger.json`, and reads only what its rules need. Fields it
does not read (a store's `growth`, a new field on an objective) are ignored, never rejected: some-ui's check holds the ledger's shape, and some-ui's grammar can grow without a
change here. A `schema` other than `some-ui.assurance-ledger/1` fails, since a new grammar may
mean something this reader does not know.

1. **A proof that holds exists.** A `server:<file>#<name>` proof of an objective that holds (and
   has not retired) names a file git tracks here that contains `<name>` (at least three
   characters), the rule some-ui applies to its own `some-ui:` proofs. A renamed test or a moved
   file fails it here, on the pull request that renamed it.
2. **A path it names exists.** A `server:` path in a family's `paths`, an evidence ref or a watch
   ref is a tracked file or a directory holding one.
3. **Every background task belongs to a family.** A background task is a spawn site
   (`tokio::spawn(`, `tokio::task::spawn_blocking(`, `std::thread::spawn(`, a `.spawn(` method
   call) in file_host's own source, `apps/servers/file_host/src`, outside `src/bin` and test
   code. It belongs to the family whose `server:` paths hold the module that spawns it, by
   Rust's module layout: `server:apps/servers/file_host/src/mailbox` covers `src/mailbox.rs` (the
   module's root, where the mailbox poller is spawned) and everything under `src/mailbox/`. The
   families already name the modules their work lives in, so the rule asks nothing new of the
   ledger.

4. **Every table is a store, and every store a table** (STORAGE5, some-ui#2226; some-ui's check
   holds the mailbox and phone halves). The tables `migrations/*.up.sql` leave standing, applied
   in file-name order (`CREATE TABLE` adds, `DROP TABLE` removes, `ALTER TABLE x RENAME TO y`
   renames, comments and string literals read out first), each have a `server:<table>` entry in
   the ledger's `stores`, and every `server:` store names one of them. `TEMP` tables, sqlx's
   `_sqlx_migrations` and `sqlite_*` are not stores. The reading is lexical, so SQLite then
   applies the same files to an empty database and the check fails if the two table sets differ:
   a statement the reading misses is seen rather than passed. A new table therefore needs its
   store in some-ui's ledger, saying how it grows, before the migration that adds it merges. On
   the day this landed, the migrations left 40 tables standing and the ledger listed all 40.

### What is not a background task, and what is not placed yet

- `NOT_BACKGROUND` lists spawns that owe no later outcome, with their exact count: the shutdown
  signal forwarder in `main.rs`, and the per-connection tasks of the WebSocket, whose module the
  ledger places in `synchronous` ("owed only while it is open"). A count that changes fails, so a
  poller added to `main.rs` is seen.
- `KNOWN_UNPLACED` lists real background tasks the ledger has not placed yet. They are warnings,
  not failures: placing one is a some-ui change, and failing here would turn every server pull
  request red until it lands. One is left: `apps/servers/file_host/src/metrics/periodic.rs`, the
  10 s operational-gauge sweep. No family fits it yet; where it goes is the owner's call
  (paulgsc/some-ui#2272), and until then GC4 reads `broken`. The daily storage sampler
  (`storage.rs`), the other on the day this check landed, is in `what-is-kept` since
  paulgsc/some-ui#2271.

  Once the ledger places one, the check warns that its entry can go; it does not fail, for the
  same reason. A new unplaced task, in a file in neither list, fails.

### Where it stops

- **file_host only.** The families describe home, and home is file_host. Library crates it links
  (`some-transport`, `ws-connection`) spawn connection plumbing of their own, and the other apps
  here are not home; none of them is scanned.
- **Lexical.** Comments and strings are blanked, and an item under `#[cfg(test)]`, `#[test]` or
  `#[tokio::test]` is skipped, as is the file of an out-of-line `#[cfg(test)] mod name;`. A test
  gated any other way (`#[cfg(all(test, ...))]`) is read as served code, so it fails loudly
  rather than hiding a task. A `use` that imports `spawn` itself (`use tokio::spawn;`) fails,
  because its bare `spawn(` calls read like a local function's.
- **Which side to fix.** The job has no path filter, because some-ui's `main` moves on its own: a
  pull request here can be the first to meet a ledger that names a server path that is gone. If
  the pull request removed it, fix it here; if the ledger named it wrongly, fix the ledger in
  some-ui.

## `PR gate`

`main`'s ruleset can require checks only by name, and this repository's checks live in several
workflows, some filtered by path. A job cannot `needs` a job in another workflow, so the gate
waits on the head commit through the API:

1. **What should run.** It reads every workflow file the pull request's merge commit holds and,
   for each with a `pull_request` trigger, applies that trigger's `branches` and `paths` filters
   (GitHub's pattern syntax, in order, `!` negating) to the base branch and the changed files.
   Those runs are expected.
2. **What runs.** The latest workflow run per workflow file for the head commit (events
   `pull_request`, `pull_request_target` and `dynamic`, which is how CodeQL's default setup runs),
   and the check runs other apps report on it (code scanning's `CodeQL`). All are awaited,
   expected or not, except the gate's own workflow.
3. **The verdict.** It passes once every expected workflow has a run and every awaited run and
   check run concluded `success`, `skipped` or `neutral`. It fails as soon as one concludes
   otherwise, naming the failed jobs, and fails what is still unfinished after 60 minutes. A run
   is judged by its own conclusion, so a job its workflow lets fail (`continue-on-error`:
   `deny.yml`'s advisories) is a warning, as that workflow's author chose.

**A skipped check passes only where its paths did not change.** A workflow its `paths` filter
skips has no run at all: the gate reads the same filter, so a run it expects and never sees fails
it (after ten minutes; GitHub creates an event's runs before any job starts), and one it does not
expect is not waited on. A job skipped inside a run passes, which is right because every `if:` on
a pull-request job here reads a path diff (`determine_jobs`) or the event (`sync`, `stage`, which
are not pull-request jobs). That is an assumption, not a check: **a new job-level `if:` on a
pull-request job that skips for any other reason would pass the gate while skipped**, so a
reviewer reading one should ask what skips it.

**Not itself.** The gate never waits on its own workflow file, and that file holds only the gate:
the script fails if its own run has a second job, which nothing would wait on.

**Where it stops.**

- After "Re-run failed jobs" on another workflow, re-run `PR gate` too; nothing starts it again.
- On a pull request of more than 300 files GitHub reads only 300 for a path filter, so an expected
  path-filtered run that never comes is a warning there, not a failure.
- If GitHub starts a workflow the gate's reading of its filter says it would not, the gate waits
  on it anyway and warns that `scripts/pr_gate.py` reads that filter wrongly.
- A trigger written in a shape the gate cannot read (an inline `pull_request: {...}`) fails it,
  asking for block style.

The check's name is `PR gate`. Once `main`'s ruleset requires it in place of the five named
checks, some-ui's `packages/assurance/src/branch-rules.ts` names it too.

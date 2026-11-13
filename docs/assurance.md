# The assurance ledger, from this side

paulgsc/some-ui keeps one record of what the system promises about outcomes and work: the
assurance ledger (`packages/assurance/src/ledger.json` there; its grammar is some-ui's
`docs/operational-assurance.md`, its epic some-ui#2199). Each promise (an *objective*) has a
verdict, and one that `holds` names the proof that holds it, as `<repo>:<file>#<name>`.
some-ui's own check holds everything it can see, which stops at this repository. Two jobs
here hold the rest (#526):

- **`Assurance ledger`** (`.github/workflows/assurance.yml`, `scripts/check_assurance_ledger.py`):
  what the ledger says about this repository is still true here (objective GC4).
- **`PR gate`** (`.github/workflows/gate.yml`, `.github/workflows/gate-verdict.yml`,
  `scripts/pr_gate.py`): one check that passes only
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
reads the head commit through the API and writes its verdict as a check run named `PR gate`.

**No runner waits (#762), but one (#814).** The gate used to be one job that polled the other checks
for up to an hour, holding a runner from the pool both repositories share. Now two short jobs judge
the head commit once each, and write what they see, pending or decided. The one wait left is a
dispatched verdict's, on a bot-pushed head, for a check that will tell no one when it finishes, and
only once nothing else is at work (below): there the alternative is a gate nobody decides.

- `gate.yml`'s `PR gate (open)`, on the pull request's own events (a push, or a label that re-runs
  `routes.yml`), turns the gate pending at once;
- `gate-verdict.yml`'s `PR gate (verdict)`, on every completed workflow run it lists
  (`workflow_run`), so the last workflow to finish decides the gate, and a re-run of a failed one
  decides it again;
- `gate-verdict.yml`'s `PR gate (sweep)`, every 15 minutes, sends each open pull request whose gate
  is still unfinished back to `PR gate (verdict)` (`workflow_dispatch`). Other apps' checks (code
  scanning's `CodeQL`) reach no workflow here, since GitHub fires no `check_run` for a head GitHub
  Actions runs on, so this judges one that finishes after the last workflow, and any event GitHub
  drops. It judges nothing itself, so every write stays inside the head's concurrency group. **It is
  best effort, not a bound:** GitHub's scheduler skips most of its slots here (7 runs in its whole
  history by 2026-11-12, the last three 5 to 7 hours apart), so no path counts on it.
  It costs 96 runs a day of about 10 s each, plus one ~15 s judging job per gate still unfinished.
  To sweep now, run `PR gate verdict` by hand (Actions → Run workflow) with its inputs empty; with
  `pr` and `head_sha` it judges that one head instead (#772).
- each pull-request workflow's last job, `Tell PR gate` (`tell_gate`), on a pull request the image
  bot pushed. GitHub fires no `workflow_run` when a run whose actor is `github-actions[bot]`
  finishes (events a `GITHUB_TOKEN` causes start no workflow, `workflow_dispatch` and
  `repository_dispatch` aside), so `PR gate (verdict)` never heard those runs, and the schedule was
  the only thing that could decide their gate (#772, #781). So the job, which `needs` every other
  job and runs `if: always()` only when the actor is that bot and the head is this repository's (a fork's
  token cannot dispatch; the sweep judges those), sends the head to `PR gate (verdict)`
  through `workflow_dispatch`, the one event that token may fire. Its run is still finishing when
  the verdict starts, so a dispatched verdict first waits, up to 3 minutes (seconds, in practice),
  for each run on the head whose only unfinished job is `Tell PR gate`. Then, if no pull-request
  run is still at work, it waits for what tells no one when it finishes (#814): CodeQL's `dynamic`
  run, whose finish fires no `workflow_run` for that actor either and which no `tell_gate` can
  follow, and other apps' check runs, until each finishes or is an hour old (the gate fails it
  then anyway). On server#813 CodeQL's Rust analysis ended two minutes after every workflow, and
  the gate sat pending 25 minutes until someone swept by hand. A run still at work is never waited
  on: it tells the gate itself. Everyone else's runs skip the job. The gate fails a pull
  request whose workflow lacks it or whose `needs` leaves a job out.

They share one concurrency group per head commit, so the writes land in the order they were read.
A write that changes a finished verdict writes a new check run, which is the one the ruleset reads,
as a re-run job's is.

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
   otherwise, naming the failed jobs, and fails a run that took over 60 minutes (what the waiting
   gate failed as unfinished after 60). A run
   is judged by its own conclusion, so a job its workflow lets fail (`continue-on-error`:
   `deny.yml`'s advisories) is a warning, as that workflow's author chose.

**A skipped check passes only where its paths did not change.** A workflow its `paths` filter
skips has no run at all: the gate reads the same filter, so a run it expects and never sees fails
it once nothing else is unfinished (GitHub creates an event's runs before any job starts), and one
it does not expect is not waited on. A job skipped inside a run passes, which is right because every `if:` on
a pull-request job here reads a path diff (`determine_jobs`) or the event (`sync`, `stage`, which
are not pull-request jobs), except `tell_gate`'s, which reads the actor and checks nothing. That is an assumption, not a check: **a new job-level `if:` on a
pull-request job that skips for any other reason would pass the gate while skipped**, so a
reviewer reading one should ask what skips it.

**Not itself.** The gate never waits on `gate.yml`, and that file holds only the gate: the script
fails if its latest run has a second job, which nothing would wait on, whichever job judges. `gate-verdict.yml` runs on events
the gate never awaits.

**Every workflow is heard.** `gate-verdict.yml` hears a workflow finish only if its
`workflow_run` names it, so the gate fails a pull request whose workflows include one that list
leaves out, and the script's self-test fails a name listed there that no workflow has.

**Where it stops.**

- `workflow_run`, `schedule` and `workflow_dispatch` run the default branch's copy of `gate-verdict.yml` and the
  script, so a pull request that edits the verdict is judged by `main`'s. `gate.yml` still runs the
  pull request's own copy, as the gate always did, so a pull request that edits its own gate can
  still get through (GC5's exception).
- GitHub runs the base branch's `gate-verdict.yml`, so a workflow this pull request adds or
  renames is one it cannot hear yet (and none, on the pull request that first adds it). Then
  `gate.yml` waits in the job as before (up to 60 minutes), since a workflow nothing hears could
  finish last and leave the gate pending.
- A fork's pull request gets a read-only token on `pull_request`, so `PR gate (open)` cannot write;
  `PR gate (verdict)` finds the pull request by its head branch and writes the verdict.
- A check that finishes after the last workflow, and that no event reports, is judged at the next
  sweep: pending, never green, until then. GitHub's scheduler skips most sweep slots here (three
  runs in 13 hours on 2026-11-10), so "the next sweep" can be hours: that is why the bot path does
  not lean on it, and a dispatched verdict waits for CodeQL instead (#814). On a human's pull
  request CodeQL's `dynamic` run fires `workflow_run`, so its finish judges the gate itself.
- A pull request the image bot (`detect.yml`'s `stage`, `GITHUB_TOKEN`) opens or pushes gets its
  `pull_request` runs in an approval-required state (`action_required`): someone with write access
  approves them (**Approve workflows to run**), or pushes to the branch, whose runs are then theirs
  and need no approval. A GitHub App token for that push would remove both the approval and the
  need for `tell_gate`; that is a new credential, the owner's call.
- On a pull request of more than 300 files GitHub reads only 300 for a path filter, so an expected
  path-filtered run that never comes is a warning there, not a failure.
- If GitHub starts a workflow the gate's reading of its filter says it would not, the gate waits
  on it anyway and warns that `scripts/pr_gate.py` reads that filter wrongly.
- A trigger written in a shape the gate cannot read (an inline `pull_request: {...}`) fails it,
  asking for block style.

The check's name is `PR gate`. Once `main`'s ruleset requires it in place of the five named
checks, some-ui's `packages/assurance/src/branch-rules.ts` names it too.

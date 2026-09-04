# CLAUDE.md

## Pre-commit verification

`.husky/pre-commit` runs `pnpm dlx lint-staged` unconditionally (no `CLAUDE_CODE_REMOTE`
guard, unlike `paulgsc/some-ui`'s hook) — but `lint-staged` here only covers this repo's
JS/TS-adjacent files. It says nothing about the Rust workspace: nothing in the git hooks
invokes `cargo` at all, in any environment. A clean, quiet commit is not evidence the Rust
side compiles, passes clippy, or has an up-to-date `.sqlx/` cache — run
`cargo check --workspace`, `cargo test --workspace`, **`cargo clippy --workspace` (see flags
below — this is not optional; `cargo check` passing is not evidence `cargo clippy` will,
verified directly: `cargo check -p enum-name-derive` succeeds while `cargo clippy -p
enum-name-derive --keep-going --no-deps` reports real denied-lint errors in that crate's own
non-test code)**, and `cargo sqlx prepare --workspace` yourself before every push, regardless
of environment. `sqlx::query!`/`query_as!` verify SQL against a real database at compile
time, so `DATABASE_URL` must point at a migrated throwaway database, not be unset.

For `cargo clippy` specifically: pass `--keep-going`, since cargo's default fail-fast
scheduling stops checking a second crate the moment the first one errors, silently
truncating the finding list to whatever crate happened to fail first. Redirect output with
`>`, never pipe through `| tail` — a pipeline reports the last command's exit code, not
clippy's, so a piped run can look clean when it wasn't. **Also pass `--all-targets`** —
without it, cargo doesn't compile `#[cfg(test)]` code at all, so clippy never even sees test
modules, let alone lints them (verified directly: `cargo clippy -p activity_repo --no-deps`
compiles clean even with a real `.expect()` sitting in its own test module; adding
`--all-targets` to the identical command surfaces 27 errors in that same crate, `.expect()`
included). CI's own `lint.yml` clippy job never passes `--all-targets` either, so it is
currently blind to every clippy issue in test code — this is the only place that discipline
gets enforced at all.

## Cross-repo coupling with `paulgsc/some-ui`

This repo's generated route snapshot (`dump-routes`, `apps/servers/file_host/docs/route-inventory.md`)
is consumed read-only by `paulgsc/some-ui` as `packages/contract-harness/routes.server.json`
and `packages/server-routes/src/generated/routes.ts` — both are copies of an artifact this
repo generates about itself, not files `some-ui` owns the content of. `RouteDescriptor` only
carries method, path, the versioning flag, and module — it proves a route exists and where,
nothing about its request/response shape. When a change here adds, removes, renames, or moves
a route, regenerate and hand off the new snapshot so `some-ui`'s contract harness catches the
diff; a payload-only change won't show up in this artifact at all, so don't treat a clean
route-snapshot diff as proof a payload change made it across — see that repo's own `CLAUDE.md`
for how it consumes the snapshot.

## Cold-start footguns worth not re-discovering

These bite during ordinary implementation work, **before any PR exists** — read this at the
start of a session, not only once you're driving a PR's CI/review cycle (that part is
`.claude/skills/steward/SKILL.md`, which only fires once a PR is open).

This section — and `steward/SKILL.md` and `babysit/SKILL.md` — is living, not archival. A
future session may add a footgun once it's actually recurred or bitten, and may remove one
that turns out to be stale, wrong, or not worth the bloat it costs every session that reads
this file. Hold every change to the bar the current entries meet: recurs across more than
one session, or is a single incident with a silent failure mode and a near-free guard —
never "sounds like good practice."

- **No `sqlite3` CLI installed.** Use Python's built-in `sqlite3` module for manual
  inspection of a throwaway/migrated database instead.
- **`.expect()` in test code is not covered by `clippy.toml`'s `allow-unwrap-in-tests`** —
  that setting exempts `.unwrap()` only, and `.cargo/config.toml`'s trailing `-D warnings`
  does promote the `-W clippy::expect_used` flag on that same list to a hard error (verified
  directly against a real crate — see the `--all-targets` note in "Pre-commit verification"
  above for why this only shows up with that flag). Use `.unwrap()`, never `.expect()`, in
  test code — `cargo clippy --all-targets` is the only thing that will ever catch this here;
  CI's own clippy job won't.
- **A tool call can be denied by this environment's permission classifier independent of
  whether the action itself is valid.** Scheduling and cleanup calls in particular
  (`send_later`/`create_trigger`, `unsubscribe_pr_activity`, `delete_trigger`) have each been
  denied in some sessions and succeeded immediately in others — no call is reliably
  always-blocked or always-unblocked. Retry a denied call at most once or twice; if a closely
  related tool does functionally the same thing, try that once before concluding the
  capability is unavailable this session. If a blocked call actually matters, say so rather
  than silently working around it.
- **Before your first commit on a designated branch, check whether that branch's most recent
  PR already merged.** If so, restart the branch from a freshly fetched default branch
  (`git fetch origin main && git checkout -B <branch> origin/main`) before adding new
  commits — never stack new work on already-merged history. Fetch fresh; a stale local
  `origin/main` ref produces false alarms in both directions.
- **Run `git diff --cached --stat` before every commit, not only `git status`.** Run it
  after staging (`git add`) — plain `git diff --stat` only shows the unstaged worktree, so
  it can miss binary corruption in content that's already staged and about to be committed.
  A `Bin ... -> ... bytes` line on a file you expect to be text source is the tell for
  embedded-NUL or other binary corruption that no lint, typecheck, or test will catch.

## Multi-session relay work

Some stories in this repo span many sessions, each picking up from a self-contained handoff
document the previous session wrote — a new session has no memory of prior conversation and
must be able to work from the handoff alone. If you're continuing one, re-read the live
issue/PR it describes before trusting its summary; it may have moved on since the handoff
was written. A footgun that recurs across more than one handoff belongs promoted into this
file instead of left for whichever future session happens to receive that specific handoff —
that's what the section above is.

If you're leaving unfinished multi-session work: write the next handoff from
`.claude/skills/steward/handoff-template.md` and deliver it to the user directly (e.g. via
`SendUserFile`) rather than committing it.

See `.claude/skills/steward/SKILL.md` for how to drive an already-open PR (auto-merge
mechanics, bot-review handling, the re-review-request idiom) and
`.claude/skills/babysit/SKILL.md` for the separate polling-cadence policy — both are
consulted automatically when acting on CI or review events, not just on request.

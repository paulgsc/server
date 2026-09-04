# CLAUDE.md

## Pre-commit verification

`.husky/pre-commit` runs `pnpm dlx lint-staged` unconditionally (no `CLAUDE_CODE_REMOTE`
guard, unlike `paulgsc/some-ui`'s hook) — but `lint-staged` here only covers this repo's
JS/TS-adjacent files. It says nothing about the Rust workspace: nothing in the git hooks
invokes `cargo` at all, in any environment. A clean, quiet commit is not evidence the Rust
side compiles, passes clippy, or has an up-to-date `.sqlx/` cache — run
`cargo check --workspace`, `cargo test --workspace`, and `cargo sqlx prepare --workspace`
yourself before every push, regardless of environment. `sqlx::query!`/`query_as!` verify
SQL against a real database at compile time, so `DATABASE_URL` must point at a migrated
throwaway database, not be unset.

For `cargo clippy` specifically: pass `--keep-going`, since cargo's default fail-fast
scheduling stops checking a second crate the moment the first one errors, silently
truncating the finding list to whatever crate happened to fail first. Redirect output with
`>`, never pipe through `| tail` — a pipeline reports the last command's exit code, not
clippy's, so a piped run can look clean when it wasn't.

## Cross-repo coupling with `paulgsc/some-ui`

This repo's generated route snapshot (`dump-routes`, `apps/servers/file_host/docs/route-inventory.md`)
is consumed read-only by `paulgsc/some-ui` as `packages/contract-harness/routes.server.json`
and `packages/server-routes/src/generated/routes.ts` — both are copies of an artifact this
repo generates about itself, not files `some-ui` owns the content of. When a change here
touches a route's request/response shape, regenerate and hand off the new snapshot rather
than letting `some-ui` drift from what this repo actually serves; see that repo's own
`CLAUDE.md` for how it consumes the snapshot.

## Cold-start footguns worth not re-discovering

These bite during ordinary implementation work, **before any PR exists** — read this at the
start of a session, not only once you're driving a PR's CI/review cycle (that part is
`.claude/skills/steward/SKILL.md`, which only fires once a PR is open).

- **No `sqlite3` CLI installed.** Use Python's built-in `sqlite3` module for manual
  inspection of a throwaway/migrated database instead.
- **`.expect()` in test code is not covered by `clippy.toml`'s `allow-unwrap-in-tests`** —
  that setting exempts `.unwrap()` only. Use `.unwrap()`, never `.expect()`, in test code, or
  a plain `cargo check`/`cargo test` run will look clean while `cargo clippy` still fails.
- **A tool call can be denied by this environment's permission classifier independent of
  whether the action itself is valid** — scheduling calls (`send_later`/`create_trigger`)
  and cleanup calls (`unsubscribe_pr_activity`/`delete_trigger`) have each been denied in
  some sessions and succeeded immediately in others; no call is reliably always-blocked or
  always-unblocked. Treat each denial as independent: retry at most once or twice, and if a
  closely related tool does functionally the same thing, try that once before concluding the
  capability is unavailable this session. If a blocked call actually matters (state that
  should have been cleaned up wasn't, or there's no way left to get a future check-in
  scheduled), say so rather than silently working around it or silently doing without.
- **Before your first commit on a designated branch, check whether that branch's most
  recent PR already merged** (`git log`, or check the PR's state) — if so, restart the
  branch from a freshly fetched default branch (`git fetch origin main && git checkout -B
  <branch> origin/main`) before adding new commits; never stack new work on top of
  already-merged history. Always `git fetch origin main` fresh before trusting a local
  `origin/main` ref for this check — a stale ref produces false alarms in both directions.
- **Run `git diff --stat` before every commit, not only `git status`** — a `Bin ... -> ...
  bytes` line on a file you expect to be text source is the tell for embedded-NUL or other
  binary corruption that no lint, typecheck, or test will catch.
- **Verify a generated/derived value against the running code before asserting it**, don't
  hand-derive the expected value and trust it uninspected — anything with non-obvious
  ordering or filtering rules (a formatted string, a serialized summary) is easy to
  hand-predict wrong and only catch in front of a reviewer.

## Multi-session relay work

Some stories in this repo span many sessions, each picking up from a **self-contained
handoff document** written by the previous one — a new session has no memory of prior
conversation and must be able to work from the handoff alone. If you're continuing one:
re-read the live issue/PR the handoff describes before trusting its summary — it may have
been edited, or the state may have moved on, since the handoff was written. Don't assume
the footguns above are exhaustive either — a handoff documenting a *new* one is worth
folding back into this file rather than left to be rediscovered by whichever future
session happens to receive that specific handoff.

If you're leaving unfinished multi-session work at the end of a session: write the next
handoff from `.claude/skills/steward/handoff-template.md`, and deliver it to the user
directly (e.g. via `SendUserFile`) rather than committing it to the repository. A handoff is
a stopgap for what hasn't earned a place in this file yet, not a replacement for it — a
footgun that shows up in more than one handoff belongs here instead, where every session
sees it regardless of which handoff (if any) it was actually handed.

See `.claude/skills/steward/SKILL.md` for how to drive an already-open PR (auto-merge
mechanics, bot-review handling, the re-review-request idiom) and
`.claude/skills/babysit/SKILL.md` for the separate polling-cadence policy — both are
consulted automatically when acting on CI or review events, not just on request.

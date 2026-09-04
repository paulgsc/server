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

## Multi-session relay work

Some stories in this repo span many sessions, each picking up from a **self-contained
handoff document** written by the previous one — a new session has no memory of prior
conversation and must be able to work from the handoff alone. If you're continuing one:
re-read the live issue/PR the handoff describes before trusting its summary — it may have
been edited, or the state may have moved on, since the handoff was written.

If you're leaving unfinished multi-session work at the end of a session: write the next
handoff from `.claude/skills/steward/handoff-template.md`, and deliver it to the user
directly (e.g. via `SendUserFile`) rather than committing it to the repository.

See `.claude/skills/steward/SKILL.md` for how to drive a PR (auto-merge mechanics,
bot-review handling, the re-review-request idiom) and `.claude/skills/babysit/SKILL.md`
for the separate polling-cadence policy — both are consulted automatically when acting on
CI or review events, not just on request.

When a test or doc asserts a value computed by generated or derived output (a formatted
string, a serialized summary, anything with non-obvious ordering or filtering rules),
verify the prediction against the actual running code before writing it into the
assertion — don't hand-derive the expected value and trust it uninspected.

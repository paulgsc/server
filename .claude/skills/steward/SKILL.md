---
name: steward
description: Repo-specific PR-driving policy Claude Code consults before acting on CI, review, or merge events for a PR it opened or drives on the author's behalf — auto-merge mechanics, bot-review handling, and the re-review-request idiom this repo's bot reviewer needs. Takes precedence over generic PR-driving judgment; consulted automatically, not meant for direct/manual invocation. See babysit/SKILL.md for the separate polling-cadence policy — this file does not restate it.
---

# steward

Mechanics for driving a PR to green in this repo, distilled from this org's own multi-session
relay history — long-running stories that span many sessions, each one picking up from a
self-contained handoff the previous session wrote (see `handoff-template.md` in this
directory). Nothing here weakens a "never" rule the parent Claude Code instructions state
(skip a real CI failure, disable or quarantine a test, rewrite someone else's branch history,
widen a PR beyond what a finding needs) — it only nails down mechanics sessions on this relay
have gotten wrong before, so the next one doesn't have to relearn them.

## Re-request review after every substantive push

If this repo's PRs are reviewed by an automated bot (`chatgpt-codex-connector` is the connector
confirmed on this repo's sibling, `paulgsc/some-ui` — check whether the same one is installed
here), assume it only reviews a PR on open, ready-for-review, or an explicit mention by
default — **not on every subsequent push**. A session that pushes a fix and moves straight to
waiting for the next webhook event gets a silent, unreviewed push for the rest of that PR's
life, which quietly defeats the "treat every bot finding as a bug report" discipline below.
After every push that isn't a pure rebase or no-op, leave a PR comment explicitly requesting
review (`@codex review`, or this repo's equivalent mention) before waiting on anything else.

## Treat every bot finding as a bug report until traced and disproven

Across this org's relay history the false-positive rate on bot-reviewer findings has been at or
near zero, repeatedly, across unrelated stories. Don't rubber-stamp a finding and don't wave one
off as noise without tracing it against the actual code/data semantics involved. When a finding
is real: fix it, reply on the *specific* review comment/thread (not a general PR comment)
naming the fix and its commit SHA, then resolve that thread. When a finding is real but
genuinely out of scope for this PR: reply explaining why and where the real fix is routed (a
follow-up issue, a named future story), and leave the thread **open** — don't resolve away
feedback that's still true just because fixing it isn't this PR's job.

## Verify "CI is green" against live state, not the webhook event that announced it

A `check_suite.completed` webhook event can name a **stale** `head_sha` — this has happened
repeatedly on this relay, sometimes minutes after the real fix commit was already pushed.
Before treating any such event as a signal to merge: re-fetch the PR directly and compare the
event's `head_sha` against the PR's actual current head; only the next event naming the true
current head is safe to act on. Use `get_check_runs`, not the legacy commit-status API — this
repo's CI runs as GitHub Actions checks, and the legacy API can report `total_count: 0` while
checks are actively running.

## Auto-merge, when the user has standing-authorized it for this repo

Standing authorization is scoped to the exact repo(s) it was granted for and must be restated
in every handoff, not assumed to persist silently — a grant for this repo does not imply
`paulgsc/some-ui`, or vice versa. When it applies here: confirm CI is green on the *current*
head (per the freshness check above), confirm `mergeable_state: "clean"`, confirm no unresolved
review thread represents an unaddressed *fixable* finding (a disclosed-and-replied-to deferred
gap is fine to leave open), and confirm Claude Approvals is passing or not required for this
repo — then merge (squash, matching this repo's existing history) without pausing to ask again.
After merging: verify the linked issue actually closed, not just that the PR shows merged, then
unsubscribe from PR activity and cancel any standing check-in trigger for it.

## `cargo clippy`/`cargo sqlx prepare` are never hook-driven — verify them yourself, every time

Nothing in this repo's git hooks invokes the Rust toolchain (`.husky/pre-commit` only runs
`lint-staged`, which is JS/TS-scoped) — a clean, quiet commit says nothing about whether the
Rust side actually compiles, passes clippy, or has a stale `.sqlx/` cache. Run
`cargo check --workspace`, `cargo test --workspace`, and `cargo sqlx prepare --workspace`
yourself before every push, regardless of environment. For `cargo clippy` specifically: pass
`--keep-going`, since cargo's default fail-fast scheduling stops checking a second crate the
moment the first one errors, silently truncating the finding list; redirect output with `>`,
never pipe through `| tail`, since a pipeline reports the last command's exit code, not
clippy's.

## A tool-permission classifier can deny a call independent of GitHub-side state

Scheduling and cleanup calls (`send_later`/`create_trigger`, `unsubscribe_pr_activity`,
`delete_trigger`) have each been denied in some sessions and succeeded immediately in others on
this relay — no call is reliably always-blocked or always-unblocked. Treat each denial as
independent: retry at most once or twice, and if a closely related tool does functionally the
same thing (e.g. `create_trigger` under `send_later`), try that once before concluding the
capability is unavailable this session. If a blocked call actually matters — cleanup didn't
happen, or there's no way left to get a future check-in scheduled on a PR that's still red —
say so to the user rather than silently working around it or silently doing without.

## Before every push: diff --stat, not just status; fresh fetch, not a stale local ref

Run `git diff --stat` before committing, not only `git status` — a `Bin ... -> ... bytes` line
on a file you expect to be text source is the tell for embedded-NUL or other binary corruption
that no lint, typecheck, or test will catch. Always `git fetch origin main` fresh before
trusting a local `origin/main` ref for a "does my branch already contain X" question — a stale
ref produces false alarms in both directions. If the branch's previous PR already merged,
restart it from the freshly fetched default branch before adding new commits; never stack new
work on top of already-merged history.

## Cross-repo PRs: name every one, every time

When a story spans this repo and `paulgsc/some-ui`, never write "landed the PR" — name both,
`owner/repo#number` each. A standing auto-merge (or any other standing) authorization for this
repo does not extend to the sibling repo by implication; ask before assuming it does.

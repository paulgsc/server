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

## Re-request review after every push

If this repo's PRs are reviewed by an automated bot (`chatgpt-codex-connector` is the connector
confirmed on this repo's sibling, `paulgsc/some-ui` — check whether the same one is installed
here), assume it only reviews a PR on open, ready-for-review, or an explicit mention by
default — **not on every subsequent push, including a rebase**. A rebase changes the head SHA
even when the diff content doesn't, and `babysit/SKILL.md`'s review-coverage check matches
that exact SHA — skip the request on a rebase and the PR can never satisfy that check again,
however many check-ins pass. After every push that changes the head SHA, leave a PR comment
explicitly requesting review (`@codex review`, or this repo's equivalent mention) before
waiting on anything else.

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
Before treating any such event as a signal to merge: re-fetch the PR directly
(`pull_request_read`/`get_check_runs`) and compare against the PR's actual current head. Don't
wait for a future webhook to confirm it — if the current head's own completion event already
arrived before this stale one showed up, there may be no "next" event ever again, and waiting
for one stalls a PR that's actually already green. The live check you just ran is the ground
truth; act on what it shows. Use `get_check_runs`, not the legacy commit-status API — this
repo's CI runs as GitHub Actions checks, and the legacy API can report `total_count: 0` while
checks are actively running.

## Auto-merge, when the user has standing-authorized it for this repo

Standing authorization is scoped to the exact repo(s) it was granted for and must be restated
in every handoff, not assumed to persist silently — a grant for this repo does not imply
`paulgsc/some-ui`, or vice versa. When it applies here: confirm CI is green on the *current*
head (per the freshness check above), confirm `mergeable_state: "clean"`, confirm bot-review
coverage on the current head is actually confirmed if this repo's reviewer doesn't auto-review
pushes (`babysit/SKILL.md`'s own criterion and graceful timeout apply here too — "no unresolved
thread" is not the same as "reviewed," since a review requested but not yet answered creates no
thread at all), confirm no unresolved review thread represents an unaddressed *fixable* finding
(a disclosed-and-replied-to deferred gap is fine to leave open), and confirm Claude Approvals is
passing or not required for this
repo — then merge (squash, matching this repo's existing history) without pausing to ask again.
After merging: verify the linked issue actually closed, not just that the PR shows merged, then
unsubscribe from PR activity and cancel any standing check-in trigger for it.

The `cargo clippy`/`cargo sqlx prepare` verification discipline, the tool-permission classifier
idiom (a scheduling/cleanup call denied independent of GitHub-side state — retry once or twice,
try a closely related tool once), and the git hygiene discipline (`diff --stat` before
committing, fresh `fetch` before trusting a local ref, restart a branch whose predecessor PR
already merged) apply here too — see the "Cold-start footguns" section of `CLAUDE.md`, which
isn't PR-lifecycle-specific so it lives there instead of being duplicated in this file.

## Cross-repo PRs: name every one, every time

When a story spans this repo and `paulgsc/some-ui`, never write "landed the PR" — name both,
`owner/repo#number` each. A standing auto-merge (or any other standing) authorization for this
repo does not extend to the sibling repo by implication; ask before assuming it does.

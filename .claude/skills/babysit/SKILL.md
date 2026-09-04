---
name: babysit
description: Repo-specific cadence policy Claude Code consults before acting on PR check-ins or webhook events for a PR it drives — governs when it's safe to stand down active polling once a PR goes quiet and green. Consulted automatically during PR babysitting; not meant for direct/manual invocation.
---

# babysit

Repo-specific cadence guidance for the PR-watch/babysit posture described in the parent
Claude Code instructions. This file governs how proactive and how long-lived that
polling is; it does not weaken any rule the parent instructions state as "never"
(skipping a real CI failure, walking away from a red or conflicted PR, disabling a
test, etc.) — those still apply in full.

## Stand down once a PR goes quiet and green

Scheduled check-ins on a PR that stopped changing keep spending tokens for no benefit.
Track this per PR you are watching (opened by you, driven for its author, or explicitly
subscribed to on the user's behalf).

A PR is **quiet-and-green** at a check-in when all of the following hold, compared to
the last check-in:

- CI is green on the current head, and the head commit hasn't changed since the last
  check-in,
- **bot-review coverage on the current head is confirmed, if this repo's reviewer doesn't
  auto-review pushes** (see `steward/SKILL.md`'s re-request idiom) — a clean pass reads
  like `Codex Review: Didn't find any major issues ... Reviewed commit: <sha>`, and that
  `<sha>` must match the current head, not a prior one. A review requested but not yet
  answered blocks quiet-and-green — "no new activity" isn't the same as "reviewed and
  clean." But be graceful: the reviewer itself can be down or unreachable. If a requested
  review gets no response across 2 consecutive check-ins, stop waiting on it specifically —
  note once that review coverage on the current head is unconfirmed and let the rest of
  this checklist decide, rather than polling forever on a response that may never come,
- there are no unresolved review threads that represent an unaddressed **fixable** finding,
  and no new review, review-thread, or PR-comment activity since the last check-in — an
  existing unresolved thread with no new activity still blocks quiet-and-green, unless it's
  a disclosed-and-replied-to deferred gap (`steward/SKILL.md`'s own "leave real-but-out-of-
  scope findings open" rule) — that kind doesn't block stand-down once it's been replied to;
  only a thread nobody has actually addressed does,
- no merge conflict against the base branch,
- Claude Approvals (where the repo runs it) is passing or not required.

After **3 consecutive quiet-and-green check-ins**, stand down instead of scheduling
another one:

1. Post one comment on the PR (or, for a PR you're only watching on the user's behalf,
   one message to the user instead) noting it looks stable and mergeable, and that
   you're standing down from active polling — at this point it's waiting on a human to
   merge it, not on you.
2. Call `unsubscribe_pr_activity` for that PR.
3. Do not schedule a further check-in for it.

Any of the following resets the quiet-cycle counter to zero and puts the PR straight
back into the normal drive-to-green loop: a CI transition (to red, or a fresh run on a
new head), a new commit, a new review or comment, a merge conflict appearing, or an
Approvals regression. This rule only removes _idle_ re-polling of a PR that has nothing
left to react to — it never excuses skipping or delaying a reaction to something that
actually changed. If the user or a new webhook event asks you to look at a stood-down
PR again, resume normally (re-subscribe, reset the counter to zero).

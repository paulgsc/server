# Session-relay handoff template

A handoff is written for a session with **zero memory** of this conversation — it must be
fully self-contained. This is a skeleton, not a story: fill each section from the real state of
the work and delete any section that doesn't apply. Do not commit the filled-in copy to the
repository — send it to the user directly (e.g. via `SendUserFile`) instead.

## Standing authorizations

State every standing authorization the user has granted (auto-merge, autonomous scheduling,
etc.), scoped exactly — which repo(s), under what conditions, whether it's suspended for this
session. Restate this in full every time; never let a future session infer it from the absence
of a caveat.

## What just landed

Name the PR(s) that merged since the last handoff, `owner/repo#number` and squash SHA, and the
issue(s) they closed. If the story spans repos, name every PR — never "landed the PR" for a
cross-repo story.

## In-flight PR state, if anything is still open

For any PR not yet merged when this handoff is written: `owner/repo#number`, branch, current
head SHA, CI status, whether review coverage is confirmed for that exact head, any unresolved
thread and why, and whether a check-in trigger is scheduled for it. Also record **how many
review rounds have happened since the PR opened** and whether `steward/SKILL.md`'s 5-round cap
has already been hit — a findings tally alone doesn't reveal whether that came from two rounds
or five, and a successor session needs the actual round count to know whether it's still free
to auto-request another review or has to read the converging/escalating/unclear pattern first.
If `babysit/SKILL.md` is actively watching this PR, also record **both of its counters**: how
many consecutive quiet-and-green check-ins have happened (toward its 3-cycle stand-down) and
how many consecutive check-ins have passed with a review requested but unanswered (toward its
2-check-in graceful timeout) — recording only that a check-in trigger exists loses both, and a
successor can't safely reset them (adds idle polling) or guess them (risks standing down early
or waiting past the timeout). "What just landed" only covers PRs that already merged — a
successor session needs this section
to find and resume an open drive-to-green loop without re-discovering where it left off.

## Review cycle — findings and tally

For each bot/human review finding this session traced and fixed: what it was, why it was real
(or why it wasn't, if genuinely noise), the fix, the commit SHA. Keep a running tally (real
findings / total findings reviewed, **not** PR count — a single PR can carry more than one
finding, so a findings-per-PR ratio can exceed 100% and stops meaning anything) across the
relay's history if prior handoffs kept one — it's the evidence base for "treat every bot
finding as a bug report."

## Disclosed but deliberately not fixed

Anything genuinely out of scope for the work just done: what it is, why it wasn't fixed here,
where it's routed (a follow-up issue/story), and confirmation the review thread (if any) was
replied to and left open rather than silently resolved.

## Process wrinkles this session hit

Anything a future session would otherwise rediscover the hard way: environment quirks, tool
denials and what worked instead, a stale-webhook trap actually encountered, a build/test
footgun. Prefer a concrete example over a general warning.

## Build & test setup

The exact commands to reproduce a green build locally — copy-pasteable, not paraphrased.

## PR process idioms

Commit message convention, PR title convention, closing-keyword convention, attribution
footer, merge method — whatever this repo's actual history shows, not a generic guess.

## Continuing the relay

What's next, what it depends on (confirm those dependencies are actually landed — don't trust a
prior handoff's claim without checking), and what's explicitly out of scope for it.

## Sending the next handoff

Restate the instruction to write the next handoff in this same shape, deliver it the same way,
and keep every standing authorization and running tally alive across the handoff boundary.

# The study nudge

> The session was prepared. I didn't sit down. The browser was closed. The
> desktop notified me. I clicked it and landed in the session.

That sentence is the whole feature. Everything below exists to make it true, and
— just as importantly — to make the silences correct: after studying, during
quiet hours, and while the dashboard is open.

This document covers setup, the decisions that are easy to get wrong, and, at
the end, [what this still cannot know](#known-unknowns). That last section
matters as much as the first. The premise of the feature is that the learning
process must never be load-bearing, so what is worth writing down is not that it
worked once, but what it still cannot tell you.

---

## Warrant, admissibility, actuation

The mistake this design exists to avoid is fusing three different questions into
one predicate.

| | Question | Where |
|---|---|---|
| **Warrant** | Why intervene at all? | `crates/intervention` — a charge falling to a threshold |
| **Admissibility** | May we, *now*, on this channel? | `file_host::nudge::constraints` |
| **Actuation** | How does it physically go out? | `crates/push_kit` |

A design with no warrant layer has to borrow one, and the usual loan is a cron:
something must make the call happen, so the clock does. The tell is a policy
whose every guard answers *may we* — quiet hours, cooldown, presence — and none
answers *why*. Time then becomes the cause of interventions rather than a
constraint on them, and the system can express "it is 19:00" but not "they
abandoned a session and twenty minutes have passed".

So the crates are split along that seam and not along a framework boundary:

| Crate | Knows about | Does **not** know about |
|---|---|---|
| `push_kit` | VAPID, RFC 8291/8292, a transport trait | study sessions, axum, sqlx, *when* |
| `intervention` | charge, decay, thresholds, verdicts | lessons, HTTP, databases, tokio |
| `study_domain` | lessons, sessions, scores, curriculum | how any of it is stored or sent |
| `engagement_repo` | rows | what a class means |
| `file_host` | axum | all of the above, except as dependencies |

There is no `dyn` in `push_kit` or `intervention`. Not as dogma — a binary knows
its transport and its domain at compile time, so a vtable to reach one
implementation buys nothing and costs inlining. Where the inverse would buy
something, it would be defensible; here it does not.

## Scaling invariants, and drift is loud

The nudge is **memoryless**. Every per-subject state is a sink: a subject's gate
row stays exactly as it is until an outside event moves it — the subject doing
something (a signal folds in and re-solves `eligible_at`), or the world changing
(new material moves the curriculum epoch). The waker discovers what the
arithmetic already decided; it never scans, never decays anyone on a timer, and
never keeps books about who has been told what. Four invariants make that true
and keep it true as the system grows:

1. **O(1) work per signal emitted.** Publishing new material is one append to
   `curriculum_publication`, whatever the number of subjects. Nothing is fanned
   out on write.
2. **O(1) state per subject, independent of the number of signals.** A subject
   carries a charge, a gate row, and one watermark per kind of global event
   (`engagement_gate.curriculum_epoch`). **No table has one row per (signal,
   subject)** — that product is the shape of a notification system that has
   turned into a database engine.
3. **The waker's only query is an indexed range read** (`EngagementRepository::due`):
   subjects whose `eligible_at` has passed, or whose watermark is behind the
   epoch. A quiet day returns nothing.
4. **Every pass is bounded by `BATCH` and the pass deadline, and nothing else.**
   A mechanism that needs its own per-pass cap, cursor, or deadline checks is
   doing fan-out work the gate should have made unnecessary.

**Drift is loud, not silent.** When a story's spec, a change, or a fix for a
review finding would break one of these, **stop and raise it** on the issue or
PR — name the invariant and the shape that would break it — instead of
implementing it or patching around it. The tell is a review cycle whose
findings cluster in one mechanism (deadline inside the fan-out, then the
cursor, then the audience snapshot…): each fix is locally right and the shape
is wrong. #273 is the worked example. Its original spec asked for a
per-subject fan-out; the first implementation built one faithfully, four
review rounds kept finding holes in exactly that mechanism, and the answer was
not a fifth fix but the epoch and watermark in "New material is one epoch, not
a fan-out" below. Changing an invariant is allowed — by deciding to, in this
section, not by accretion.

---

## The battery, and why it is not a rate limiter

Engagement is a **vector** of levels, one per class, that decays with time and is
restored by signals. Decay *is* inactivity — there is no separate "days since
last seen" counter, because the absence of signals is already the drain.
Discrete setbacks drain further; wins recharge.

An intervention is warranted when the weighted aggregate falls to the threshold.

The analogy stops at the leaky bucket, and this is the part worth understanding:

- **Nothing is polled.** Decay is closed-form, so a level is computed on read and
  never ticked. A subject nobody has seen in a year costs one exponential when
  someone finally asks.
- **The crossing instant is solved, not waited for.** Between signals the
  aggregate is strictly decreasing, so the moment it will cross the threshold can
  be found by bisection — once, when the signal arrives — and written to an
  indexed column.
- **The waker therefore discovers rather than decides.** Its entire query is
  `WHERE eligible_at <= now`. On a day when nobody has drifted it returns
  nothing.

Work is O(1) per signal and zero per idle subject. `NUDGE_WAKER_SECONDS` is not a
schedule; it is the resolution at which already-decided work is picked up.

### Why a vector and not one number

A scalar can say *whether* to intervene and can never say *what to say*.
"Abandoned twenty minutes ago" and "gone for a fortnight" reach the same
threshold and want completely different messages. The dominant deficit picks the
action:

| Most depleted | Action |
|---|---|
| Presence | `LessonReady` |
| Momentum | `ResumeAbandoned` |
| Mastery | `SuggestReview` |
| Freshness | `NewMaterial` |

### Cold start: the one deficit with a sessionless answer

Three of the four actions above need a prepared session — you cannot resume a
session that was never started, review material that was never studied, or
announce new material to someone who has seen none. Before `#279` (RCM2),
nothing was ever prepared for these three, so they stayed silent exactly as
the table implies: `StudySelector::select` returned `None` and the engine
reached `Verdict::NothingToSay`, logged with a `warn!` and retried six hours
later, forever, for anyone whose dominant deficit was not `Presence`.

Plain absence is different: it has an honest sessionless answer. When
`prepared_session` is `None` and the dominant deficit is `Presence`, the
selector returns `StudyAction::GetStarted` instead of staying silent — an
invitation, deep-linking to the app base rather than a session that does not
exist. This was a deliberately **interim** answer, not the recommender: it
invites, it does not propose. `#279` left it alone — see "Provisioning"
below for why — and `#285` (RCM8) settled it, for the second of the two
options this paragraph used to offer: `GetStarted` does not retire, it
**narrows to the fallback for a catalogue that can compose nothing**. Every
warranted subject with nothing prepared now gets a real proposal instead, the
`Presence`-dominant one included — see "Nothing to say becomes a bug report"
below.

### Provisioning: `NothingToSay` becomes an opportunity, not a dead end

`#279` (RCM2) is what closes the gap the paragraph above describes, for the
three deficits that are not `Presence`. When `nudge::waker::consider`
reaches `Verdict::NothingToSay` — which, by the time it gets there, has
already proven the subject is due, past refractory, and has a dominant
deficit of `Momentum`, `Mastery`, or `Freshness` with nothing to resume,
review, or announce — it writes a minimal `Draft` session (no activities, no
scenes) for that subject on the spot, rather than logging a warning and
waiting six hours to ask again.

Two things below have moved since, and are described where they landed rather
than rewritten here. `#282` (RCM5) is what makes the written row a full
`Scheduled` session instead of a minimal `Draft` one — see "Materialising a
session". `#285` (RCM8) is what widens the *condition*: the write no longer
hangs off `Verdict::NothingToSay` at all, but off "warranted, with nothing
prepared", which is what finally brings the fourth deficit in — see "Nothing
to say becomes a bug report". Everything between those two sentences —
what is written, why here and not in `intervention`, and what it costs on a
crash — is unchanged by either.

Three shapes for this were weighed, and the choice is recorded in
`nudge::waker::consider`'s own doc comment on the `NothingToSay` arm, not
just here:

1. **A fifth `StudyAction` variant** (`ProposeSession`) — rejected: every
   existing variant carries a real session id, and a variant that could not
   would break `StudyAction::session_id()`'s totality.
2. **A new `Verdict` arm in `intervention`** — rejected: it would put "the
   domain wants something created" into the generic engine, which
   `intervention`'s own module docs are explicit about keeping free of study
   vocabulary.
3. **Provisioning before selection, inside the waker** — chosen. A session
   gets written, `prepared_session` becomes `Some`, and the *existing*
   `StudySelector` maps the same dominant deficit to `ResumeAbandoned`,
   `SuggestReview`, or `NewMaterial` exactly as it would for a session that
   already existed. `intervention` and `study_domain` are both untouched.

What goes *in* the provisioned session was deliberately not this story's
problem: RCM3 (`#280`, the recommender), RCM4 (`#281`, floor durations), and
RCM5 (`#282`, materialising catalogue rows into a full `SessionRecord`) are
what fill it in — see "Materialising a session" below for what RCM5
actually landed. `#283` (RCM6) is what marks a provisioned session `origin:
system` — see "Origin: `user` vs `system`" below for what that field means
and what reads it.

Crash safety costs nothing extra: the write is a `Scheduled` row (RCM5's own
choice, see below) `first_prepared` will find on any later pass, so a crash
between provisioning and `EngagementRepository::claim` costs that pass's
notification, not a second session — the next pass finds the row already
written and reaches `Verdict::Intervene` directly, without provisioning
again. The provisioned action still has to clear the same
`StudyConstraints::admit` gate as any other — quiet hours and consent apply
whether the action came from an existing session or one just created.

### Recommendation: `recommend(subject, k)` and its three axes

`#280` (RCM3) is what `#279` deferred: what actually goes into a provisioned
session. `activity_repo::recommender::recommend` takes a subject, `k`, a
bounded catalogue read (see `ActivityRepository::list`), whatever the caller
knows of this subject's per-activity history (an empty slice until `#258`
exists), an optional `last_session_at`, and a clock — and returns the `k`
best activities, pure and deterministic over all of it: no I/O, no
`rand::thread_rng`.

Three axes, applied in order:

1. **Newness dominates** (`WEIGHT_NEWNESS = 8.0`) — an activity this subject
   has never played outranks everything else; one they have played counts as
   new again only if its catalogue entry was published after their last
   session. Until `#258` supplies real play history, every activity reads as
   "never played," which is the correct cold-start default, not a
   degenerate one.
2. **Low engagement lifts, it never lowers** (`WEIGHT_ENGAGEMENT = 4.0`) — an
   abandoned attempt or a poor score raises a candidate's rank rather than
   sinking it: unfinished is worth finishing, unlearned is worth repeating.
   The gap between the two weights means axis 2 can never outvote axis 1,
   the same "no axis outvoted by the sum of those below it" discipline
   `StudyCalibration` and the client's own `rankActivitiesWithScores`
   already use.
3. **Otherwise, a seeded shuffle** — Fisher-Yates over a `StdRng` seeded from
   `(subject, day)`, so two candidates tied on both axes above — the common
   case for a zero-history subject, where every candidate ties — still get a
   stable order that changes daily rather than a slot machine.

`maturity = Early` candidates are excluded outright before any of the above
runs — a construction zone is a poor thing to propose unprompted, unlike a
browsable menu where it can just rank low.

This deliberately disagrees with the client's own `rankActivitiesWithScores`
(`packages/activity-catalog/src/lib/rank.ts`, `paulgsc/some-ui`), which
weights recency of *play* highest — correct for "what do I open right now"
on a dashboard, wrong for "what should I do next" from someone who asked for
nothing. Both sides carry a note pointing at the other, so neither gets
"corrected" to match it.

Wiring `recommend()`'s output into `provisioned_session()`'s
`activities: Vec::new()` (`nudge/waker.rs`) is RCM5's job (`#282`), not this
story's — `recommend()` lands as a pure function with no caller yet, the
same shape RCM4's `derive_min_duration_ms` already took before RCM5 needed
it too.

### Floor durations: `provision(activities)` and the client's own composer floor

`#281` (RCM4) is what decides how long each of `recommend()`'s picks
actually runs. The rule: a provisioned activity is scheduled at its
**floor**, never its **default** — `defaultMinutes` is tuned for someone
who already opened the composer and committed to an activity, and reusing
it for someone who has committed to nothing yet is exactly the mistake this
story exists to prevent.

**Where the floor has to live.** #272 (CAT4) already decided this server
never writes `scenes` — only `activities: [{ activityId, config }]`,
materialised into playable scenes by the client's own `sequenceScenes`
(`packages/activity-catalog/src/lib/to-scene-config`, `paulgsc/some-ui`).
That pipeline reads `config.durationMinutes` when present and falls back to
the catalogue's own `defaultConfig.durationMinutes` otherwise — so the
floor has to be written into `config`, or the client's own fallback quietly
reintroduces the default-duration bug this story exists to prevent.
`activity_repo::provisioning::provisioned_config` is `default_config` with
exactly that one key overridden; every other key (mode, difficulty, level,
category, …) passes through unchanged, the one deliberate exception to
#272's own "handed to the client verbatim" description of `default_config`.

**The client's floor is not the activity's floor.** Every write path —
Basic sequencing and the Advanced arrangement editor alike — is checked
against `DEFAULT_SESSION_DURATION_POLICY.minActivityDurationMs`
(`apps/www/src/lib/session-duration-policy/index.ts`, `paulgsc/some-ui`,
5 minutes) by the client's `checkSessionDuration`. A proposal below it would
be rejected by the client's own composer the moment someone opened it, so
the effective floor `provisioned_config` applies is
`max(activity_min, client_policy_min)`, never the activity's own minimum
alone. `activity_repo::provisioning::CLIENT_MIN_ACTIVITY_DURATION_MS`/
`CLIENT_MAX_TOTAL_DURATION_MS` transcribe that policy's two constants, and
`tests/session_duration_policy_parity.rs` is what fails if the
transcription and the client's real value ever diverge — a checked-in
fixture (`testdata/session_duration_policy.snapshot.json`), exported by
`paulgsc/some-ui`'s `scripts/dump-session-duration-policy.ts`, the same
by-hand-copy discipline `tests/catalog_parity.rs` already runs on for
`min_duration_ms` itself.

**A `NULL` `min_duration_ms` is decided, not papered over.** #269 made the
column nullable for "whichever activity is next to not need one" —
`leetype` no longer is (some-ui#1130 gave it a real duration field before
this story ever landed), but the column stays nullable and the case is
still real. The decision: an activity with no derivable floor is **omitted
from the proposal entirely**, not given a durationless block —
`provisioned_config` returns `None`, `provision` filters it out, and
`provisioning.rs`'s own doc comment names the reasoning (a block that
cannot be timed is not a block that runs for zero minutes).

**Verified against the client's real check, not a reimplementation of it.**
`#281`'s own acceptance criterion asks for a proposed session to pass
`checkSessionDuration`, "verified by a fixture consumed on the client side."
`cargo run --bin dump-proposed-session` runs `recommend()` and `provision()`
together against a literal transcription of the seeded catalogue and emits
the resulting `activities` as JSON; copied by hand into `paulgsc/some-ui`'s
`apps/www/src/lib/session-duration-policy/testdata/proposed-session.snapshot.json`,
a client-side test there runs it through the real `sequenceScenes` and
`checkSessionDuration` — the same pipeline a real device runs, not a second
copy of the rule that could quietly drift from the first. `maxTotalDurationMs`
(4 hours) is trivially true for two 5–10 minute blocks, but `provision()`
enforces it directly rather than trusting that to stay true: neither
`min_duration_ms` nor `recommend()`'s `k` has a ceiling, so it skips any
activity — including one whose own floor already exceeds the cap outright —
once including it would push the running total over
`CLIENT_MAX_TOTAL_DURATION_MS`, a real Codex review finding on server#315.

`#282` (RCM5) is what wires `provision()`'s output into a real session — see
the next section.

### Materialising a session: `name`, `total_duration_ms`, and the `scenes` decision (#282, RCM5)

`#282` closes out what `#279` (RCM2) deliberately left empty and `#280`/`#281`
(RCM3/RCM4) built as pure functions with no caller: `nudge::waker::
materialize_provisioned_session` calls `recommend()`, then `provision()`,
then fills in everything else a real `sessions` row needs.

**`status: Scheduled`, not `Draft`.** RCM2's original `Draft` choice was
defended on crash-safety grounds specific to a session with nothing in it
yet — a row born anything but `Draft` could be offered before the recommender
that would fill it in had run. That concern is retired, not just satisfied:
`recommend()`, `provision()`, and this story's naming/duration logic all run
synchronously inside `materialize_provisioned_session`, before the row is
ever written, so there is no partially-composed state a crash between
"written" and "filled in" could expose. `Scheduled` is what `#282`'s own
table asks for, and it changes nothing about how a provisioned session is
found — `SessionRepository::first_prepared` already treated `scheduled` as a
prepared status alongside `paused`/`draft`.

**`name`: `activity_repo::naming::default_session_name`.** Transcribed from
`defaultSessionName` (`packages/activity-catalog/src/lib/summary.ts`,
`paulgsc/some-ui`), the same discipline RCM4 established for the two
duration constants: distinct activity names joined with `" + "`, a repeat
labelled `"Name ×N"` rather than repeated, `"New session"` for an empty list.
Built from whichever of `recommend()`'s picks actually survived `provision()`
— an activity `provision()` omitted (a `NULL` floor, or one that would have
crossed `CLIENT_MAX_TOTAL_DURATION_MS`) has no business in the name of a
session it is not in.

**`total_duration_ms`: `activity_repo::provisioning::total_duration_ms`, not
`session_repo::total_duration_of(&scenes)`.** This is the one place RCM5
deviates from the migration comment's own claim that this column is "derived
from scenes." A provisioned session writes `scenes: []` (see below), and
`total_duration_of(&[])` reads that as zero — wrong, since the session has
real, timed blocks, only not yet materialised into playable scenes.
`total_duration_ms` sums each provisioned activity's `config.durationMinutes`
directly instead. The two numbers are provably the same for the `basic`
layout this story always produces: the client's `sequenceScenes` places
Basic-composer scenes back-to-back with no gaps and no overlap, so
`totalDurationOfScenes`'s "furthest scene end" *is* the sum of durations for
exactly this shape. See `total_duration_ms`'s own doc comment for the same
argument made where the code actually lives.

**The `scenes` decision.** `#272` already decided this server cannot
compute a scene's `props` — the closure stays client-side — so `#282`'s own
issue names three options and asks this story to pick one and defend it,
not to assume its own hint (which pointed at `paulgsc/some-ui#1038`'s PRO1)
was the final word. It wasn't: PRO1 (`some-ui#1052`) turned out, on actually
reading it, to be entirely about an `origin: "user" | "system"` field —
unrelated to scene materialisation — and none of the epic's five sub-issues
(PRO1–PRO5) mention it either. That citation in `#282`'s issue body is
stale; this section is the correction.

**Decision: `scenes: []`.** The honest option — it satisfies the schema's
`NOT NULL`, and it does not fake a `SceneConfig` shape (`ui[].panels.
mainContent.props`, in particular) the server has no way to fill in
correctly. The alternative the issue itself calls "the worst of both" —
structurally complete scenes with empty `props` — was rejected for the
reason the issue gives: it looks valid until someone opens it.

**Consequences for existing client consumers, enumerated because `#282`'s
acceptance criteria require it.** A grep of `paulgsc/some-ui` for `.scenes`
usage turns up one load-bearing consumer and several unaffected ones:

- **`components/player/live-player.tsx`** calls `configure(session.scenes)`
  directly to start playback. This is the real consequence: a person who
  hits "Start" (PRO3's own table) on a provisioned session before anything
  populates `scenes` gets a player configured with zero scenes. Every other
  consumer below is secondary to this one.
- **`components/player/completion-summary.tsx`** reads `session.scenes` to
  render a finished session's summary — only reachable after a session has
  been played, by which point scenes must already be populated (see below),
  so this consumer is unaffected as long as that holds.
- **`components/composer/session-composer.tsx`** reads `existingSession.
  scenes` only when `layoutMode === "advanced"`; a provisioned session is
  always `basic`, so this path never runs for one. Editing a provisioned
  session (PRO3) round-trips through `activities`, not `scenes`.
- **`components/composer/arrangement-step.tsx`** and **`components/player/
  use-live-layout-editor.ts`** both operate on an already-populated `scenes`
  array (Advanced editing, and live in-session layout edits); neither is
  reachable for a `basic`, not-yet-started provisioned session.

**What this means for future client work, not built by this story.** The
one load-bearing consumer needs `scenes` populated by the time a person hits
Start, and the natural, cheap way to provide that is exactly `#282`'s
Option 2: call the client's own `sequenceScenes(activities)` — the same
pure function the Basic composer already calls for a person-authored
session — before `configure()` runs, rather than trusting a `scenes` field
that a provisioned row leaves empty. That is real, if small, client-side
work, and **no currently-filed PRO-story covers it** — PRO3 (`some-ui#1054`)
names Start's existing behaviour (`status → active`, `session-started`) but
says nothing about materialising scenes first. This gap is worth a new
client story before RCM7's "one un-started proposal" bookkeeping or anyone
actually ships a Start button wired to a provisioned session; it is called
out here rather than silently assumed away.

**Verified by a fixture the client repo consumes, not a hand-copied type.**
`cargo run --bin dump-provisioned-session` calls
`materialize_provisioned_session` directly — the same function `nudge::
waker::consider` calls in production — against the same fixed subject and
seeded-catalogue transcription `dump-proposed-session` uses, and emits a
full `SessionRecord` as JSON. Copied by hand into `paulgsc/some-ui`'s
`apps/www/src/lib/tenant/testdata/provisioned-session.snapshot.json`, a
client-side test there round-trips it through the real `SessionRecord` type
and the real `sequenceScenes`/`defaultSessionName`/`totalDurationOfScenes`
functions — the same "verified against the real thing, not a
reimplementation" discipline `#281`'s own fixture already established.

### Origin: `user` vs `system`, and the abandonment guard (#283, RCM6)

`#283` closes the gap RCM5 left open on purpose: a provisioned session's
`name` was the only signal that it was proposed rather than authored, and
that signal was inferential — a person who accepts the default name without
renaming it would read identically to a proposal nobody opened.
`origin TEXT NOT NULL` (`"user"` | `"system"`) makes the distinction a real
column instead: `"user"` for every session a person composed themselves
(`POST /sessions`, and `POST /sessions/:id/duplicate` — duplicating is an
action only a person takes, see `duplicate_session`'s own doc comment for
why that overrides the plain `..source` spread every other field uses),
`"system"` for exactly what `nudge::waker::materialize_provisioned_session`
writes.

**Why a column, not an inference.** Argued in full in `#283`'s own issue
text: a person who accepts a proposal's default name and never renames it
would be indistinguishable from a proposal nobody opened, under any
name-based heuristic. Getting this backwards has a real cost — `Momentum`
would credit a proposal nobody took as an *abandoned* session, draining
engagement for someone who has not actually studied, which inverts the
entire cold-start epic (`#257`) this story belongs to.

**`SessionOrigin::parse` refuses an unrecognised value**, argued identically
to `SessionStatus::parse` (`session_repo::model`): a row holding neither
`"user"` nor `"system"` was written by something outside this schema, and
reading it as `"user"` would quietly make a proposal look authored — the
same class of mistake `SessionStatus::parse`'s own doc comment already
names for status.

**The transition is one-directional, enforced in `SessionRepository::
upsert`, not at the handler.** `system → user` is a real promotion (a
person edited or otherwise took ownership of a proposal — PRO1's own "what
counts as an edit" decision, `paulgsc/some-ui#1052`, governs when the
*client* decides to send this); `user → system` never applies, even if a
caller sends it by mistake. The SQL itself carries the guard: `origin =
CASE WHEN sessions.origin = 'user' THEN 'user' ELSE excluded.origin END` on
the `ON CONFLICT` update, so once a row is `user` no write can move it back.
There is deliberately no error for the rejected direction (unlike
`SessionRepoError::SubjectMismatch`) — silently keeping the stronger claim
is a policy outcome, not a caller mistake worth surfacing.

**`session_abandonment_is_real(origin, started_at)`** (`session_repo::
model`) is the pure predicate this story's acceptance criteria actually
needed: `false` exactly when `origin` is `system` *and* `started_at` is
`None` — a proposal nobody opened. A `system`-origin session that *was*
started is a real abandonment if paused thereafter, same as any `user`
session; starting it is a real action even though renaming it might not be.
**No caller derives a `StudySignal` from an ordinary status transition
yet** — `SessionRepository::upsert`/`set_status_many`, the only server code
paths that can drive a session's `status` today, write the column and
nothing else, and the only signal the waker itself emits is the hardcoded
`StudySignal::SessionProvisioned` in `nudge::waker::consider`. Wiring
engagement into an ordinary status `PATCH` is a real, currently-unfiled gap
— the same kind RCM5 named for materialising `scenes` before Start — so
this predicate is landed now, pure and tested, the same "function before
its caller" discipline `recommend()`/`derive_min_duration_ms` already
established, rather than left to whichever future story wires it in to get
the origin check wrong or skip it.

**A real, currently-unfiled rollout-window gap: an origin-unaware client
cannot trigger the promotion at all.** `update_session` only moves `origin`
from `system` to `user` when the patch includes an explicit `origin` field
— the repository enforces the *direction*, but nothing server-side decides
*when* to trigger it. Before PRO1 (`paulgsc/some-ui#1052`) ships, the live
client never sends that field, so a person who renames or edits a
`system`-provisioned session through today's client leaves its `origin`
unchanged in the live database, even though the exact same edit would be
correctly reclassified as `user` if it were still sitting in the table
unedited when the backfill migration ran. A real Codex review finding on
`paulgsc/server#334` caught this. The obvious-looking fix — auto-promote
`origin` on any `PATCH` that changes something — was deliberately not
taken: `update_session` is the same handler a plain status transition
(Start/Pause/Complete) goes through, and PRO1's own acceptance criteria
already name "starting a session without changing anything" as a
deliberately ambiguous case, likely *not* an edit. Auto-promoting on every
`PATCH` would flip an untouched proposal to `user` the instant someone
merely pressed Start, defeating the abandonment guard for exactly the
session it matters most for. Closing this gap needs PRO1's actual
edit-vs-lifecycle-transition boundary, not a guess made here — until then,
this is bounded and temporary: it costs a session getting `origin` wrong for
existing rows edited between #283 landing and PRO1 shipping, not a
permanently wrong invariant.

**#284 (RCM7) raised this gap's stakes from a misclassification to data
loss, and had to grow its own, narrower defence.** A `Momentum` accounting
error is silent and recoverable; overwriting a person's own edit with a
machine-generated recommendation is neither. See "Never stack proposals"
below for `refresh_stale_proposal`'s `created_at == updated_at` check —
it does not close this gap (only PRO1 can), but it does stop RCM7's own
refresh mechanism from acting on a row this gap has already misclassified.

**Payload-only, no route or contract regeneration needed.** `origin` is a
new field on the existing `SessionRecord` JSON shape, not a new route —
`docs/route-inventory.md`'s `RouteDescriptor` only proves a route exists
and where (method, path, versioning, module), nothing about payload shape,
so `dump-routes` has nothing to regenerate here. The client-side contract
update this still needs (`paulgsc/some-ui`'s hand-written `SessionRecord`
schema, #1042) is PRO1's own acceptance criterion, not this story's.

### Never stack proposals: at most one un-started system session (#284, RCM7)

`REFRACTORY` is 20 hours and `recharge_on_intervention` gives `Presence` back
30 per nudge, so a persistently disengaged subject becomes eligible again
roughly daily. Without a guard, a fortnight of ignored notifications would
produce fourteen provisioned sessions — each one a row the client lists,
each dragging the person's session list toward uselessness, and the
compounding part: `first_prepared` (#263) would keep finding *one* of them,
so the system keeps happily concluding it has something to point at while
the actual number of things the person wants to do is one.

**The rule:** before provisioning, if the subject already has an `origin =
'system' AND started_at IS NULL` session, do not create another.

**Why the predicate is `origin`/`started_at`, not `first_prepared`'s status
list.** `nudge::waker::consider` already read `SessionRepository::
first_prepared` before this story, and that query's three "prepared"
statuses (`paused`, `scheduled`, `draft`) happen to overlap with what a
never-touched system proposal looks like — which is why, in the common
case, a second row was never actually observed stacking even before this
story landed. That overlap is coincidental, not principled, and it breaks in
two places `#283` (RCM6) already named as the ones this rule must get right:

- **A `system` session the person *started* is history now, not a
  proposal.** The moment it goes `active`, it stops matching any of
  `first_prepared`'s three statuses too — `active` was never one of them —
  so relying on that list would let the waker reach `NothingToSay` again for
  a subject who is, right now, mid-session on their first proposal. `origin
  = 'system' AND started_at IS NULL` gets this right by construction:
  `started_at` moves to `Some` the instant a person presses Start and
  `UpdateSession` has no field that ever clears it back to `None`, so a
  started row is permanently excluded from this rule, whatever its `status`
  does afterward (`paused`, `completed`, or back to being re-opened).
- **A `system` session promoted to `user` by editing (RCM6's one-way
  `upsert` guard) falls out of this rule automatically.** Once `origin`
  flips, the row permanently stops matching `origin = 'system'` — RCM6's own
  guard already makes `user → system` unreachable, so there is no path back
  in. A person who renamed or edited their proposal without starting it must
  not have it silently refreshed out from under them the next time the
  waker runs; this predicate never even looks at that row again.

Both edges are asserted directly, at two layers: `SessionRepository`'s own
`provision_if_absent_never_touches_a_started_system_session` /
`_never_touches_a_system_session_promoted_to_user` tests
(`crates/db/session/src/repository.rs`), and — because this is exactly the
"interaction between two stories" #284's own issue text calls out as worth
testing — `nudge::waker`'s
`a_started_provisioned_session_does_not_block_or_get_clobbered_by_a_second_provisioning_pass`,
which drives the same scenario through `consider` end to end rather than
the repository method in isolation.

**Enforced as a real constraint, not just application-level care.**
`idx_sessions_one_unstarted_system_proposal`
(`migrations/20260910000100_at_most_one_unstarted_system_proposal.up.sql`)
is a partial unique index — `ON sessions (subject_id) WHERE origin =
'system' AND started_at IS NULL` — so the invariant holds even against a
write path this story did not anticipate, the same defence in depth
`upsert`'s own `SubjectMismatch` guard already gives a different invariant.
The migration also runs a one-time, `updated_at`-ordered dedup delete before
creating the index, for the same reason `20260906000900_add_origin_to_
sessions.up.sql` was this careful about a live deployment: `CREATE UNIQUE
INDEX` fails outright if any subject already violates it, and this should
not be the migration that discovers a violation rather than the one
introducing the rule.

**Three strategies were weighed for what happens to the existing proposal,
and the issue's own recommendation is what shipped:**

1. **Point at it again.** Cheapest — do nothing, `first_prepared` keeps
   finding the old row. Rejected: it stales. Five days on, the "new
   material" `recommend()` picked it for is not new any more, and the
   durations reflect a different day's seeded shuffle.
2. **Replace it.** Delete or supersede, then provision fresh. Rejected:
   deleting a row the person may have glanced at, or opened in a tab, is a
   small dishonesty, and `#285` (RCM8)'s own acceptance depends on an old
   notification's deep link still resolving — a new id would break exactly
   that.
3. **Refresh in place.** Same id, new contents. **Chosen** — the deep-link
   stability argument is the deciding one: `#285` needs a notification
   issued before a refresh to still resolve afterward, and only this option
   gives that for free.

**`SessionRepository::provision_if_absent`** (`crates/db/session/src/
repository.rs`) is the mechanism: one `INSERT ... SELECT ... WHERE NOT
EXISTS (...) ON CONFLICT (subject_id) WHERE origin = 'system' AND
started_at IS NULL AND status IN ('paused', 'scheduled', 'draft') DO
NOTHING` statement.

**That conflict branch used to be a `DO UPDATE`, and `#345` made it a
no-op.** Between `#284` and `#345` it rewrote `name`, `activities` and
`total_duration_ms`, guarded by `created_at = updated_at`. A real
`chatgpt-codex-connector` finding on `#342` showed that guard cannot hold
the line it was given, precisely *because* a machine write never moves
`updated_at`: two concurrent passes both reach the statement, the winner
inserts and goes on to claim and send a notification pointing at its row,
and the loser's delayed `DO UPDATE` still satisfies the guard and rewrites
that row afterwards — for a notification it never sends, since it loses the
claim moments later. The same post-send mutation race `#335`'s round 7
closed for `refresh_stale_proposal`, on the one write path that cannot be
claim-gated, because it necessarily runs before there is an action to claim.

Nothing is lost by refusing. This method is only called when
`first_prepared` returned `None`, and the conflict target is a strict
subset of what `first_prepared` searches — so a conflicting row can never
be the stale ignored proposal `#284` wanted refreshed (that one is found at
the top of `consider` and refreshed post-claim), only another concurrent
pass's brand-new one. `recommend` being deterministic per `(subject, day)`
means the two passes' content is usually identical anyway, and the cases
where it differs — the catalogue changed, or they straddle UTC midnight —
are exactly the cases where rewriting does harm. After `#345` there is
exactly one path that rewrites a proposal's content, `refresh_if_untouched`,
and it runs only after its pass has won the claim.

**Two guards, not one, because they protect against two different races.**
A first version of this statement carried only the `ON CONFLICT` — a real
`chatgpt-codex-connector` finding on `#335` caught that this drops
`provision_if_absent`'s own `WHERE NOT EXISTS (... status IN ('paused',
'scheduled', 'draft'))` guard entirely, and that guard was protecting
against a *different* race than the partial index does. The partial index
stops a second `system` proposal from ever coexisting with an un-started
one; it says nothing about a **foreign** prepared session — a real
person's own `paused`/`scheduled`/`draft` row — landing in the window
`provision_if_absent`'s own doc comment already named (#313): a concurrent
waker pass, or the subject's own `POST /sessions` call, between
`consider`'s `first_prepared` read and this write. Without the `WHERE NOT
EXISTS` restored, that race would insert a system proposal *alongside* the
person's fresh draft, and `first_prepared`'s status priority would then
surface the wrong one. The restored subquery excludes rows already matching
the partial index's own predicate, so the two guards agree on which row is
"foreign" rather than fighting over the same one — a pre-existing
`system`/un-started proposal is not foreign, it is exactly the row the `ON
CONFLICT` branch exists to yield to, leaving it untouched (`#345`).

**Bounded per #253.** Both guards are index-backed: the `NOT EXISTS`
subquery is served by `idx_sessions_status`, and the `ON CONFLICT` target
is the new partial index — an equality probe each, not a scan, the same "a
read/write reachable from the waker declares its own bound" discipline
`first_prepared`'s three single-status probes already established for the
neighbouring query.

**Refreshing has to happen somewhere other than `Verdict::NothingToSay`,
and not before admission is checked either — two real `chatgpt-codex-
connector` findings on `#335`, in sequence.** The first version of this
story gated refreshing entirely on that arm, reasoning that provisioning
always had. That never actually fires for the scenario the whole story
exists for: once an ignored proposal exists at all, `first_prepared` (read
at the top of `consider`) already resolves to it, and `StudySelector::
select` maps straight to an intervention — `NothingToSay` cannot occur
again while that row exists, so gating the refresh on reaching it would
leave every subsequent day's pass pointing at whatever `recommend()`
produced the day the proposal was first written, silently defeating the
whole point of choosing "refresh in place" over "point at it again."

The fix moved the refresh call earlier — right after `first_prepared`
resolves, before the engine ever runs — and that was itself wrong in a
different way: it could rewrite a proposal's `name`/`activities`/
`total_duration_ms` while the subject held a fresh presence lease on that
exact session, actively viewing it, only for `evaluate`'s own `admit` call
to suppress the notification on `Present` anyway. Mutating a session out
from under someone looking at it, for a notification that was never going
to send, is exactly the write race #284's own "refresh in place" choice
was supposed to avoid causing, not invite.

**`nudge::waker::refresh_stale_proposal`'s call site is the resolution to
both**: inside the `Verdict::Intervene(action)` arm of `engine.evaluate`,
not before it and not only inside `NothingToSay`. `evaluate` already calls
`Admissibility::admit` internally before ever returning `Intervene` (see
`intervention::Engine::evaluate`'s own implementation), so gating on that
verdict is sufficient by construction — no separate presence check is
needed at the call site, and refreshing is skipped automatically whenever
`Wait`, `Suppressed` (quiet hours, no consent, or presence), or
`NothingToSay` is what `evaluate` actually decided. It is best-effort
rather than fatal (a failure leaves the existing, unrefreshed proposal in
place rather than failing the pass), unlike provisioning itself, where a
failure can mean there is nothing to offer at all. Bounded the
same way: one indexed lookup by id to decide whether to refresh, not a
scan.

**Refreshing is still not "on a timer."** It only ever runs from inside
`consider`, for a subject the engagement arithmetic already marked due,
immediately before an intervention that is actually about to go out — so a
subject who is not due, not admissible, or currently viewing the proposal
gets no refresh, exactly as before; what changed is only *which*
already-due, already-admissible pass can trigger it.

**A third real `chatgpt-codex-connector` finding on `#335`, P1, is the
sharpest: `origin = 'system'` is not proof nobody has touched this row.**
The "Origin" section above already documents a real, bounded,
pre-`some-ui`-PRO1 gap — the live client never sends an `origin` field on
an edit, so `update_session` never promotes an edited proposal to `user`.
Before this story, that gap's only cost was a `Momentum` misclassification
if the row was later abandoned. A refresh mechanism turns the exact same
gap into something worse: silent data loss. A person who renames this
proposal, or replaces its activities, through today's client leaves it
reading as `origin = 'system' AND started_at IS NULL` — indistinguishable
from a genuinely untouched one by those two columns alone — so without a
further check, the next eligible pass would overwrite their own edit with
a fresh recommendation.

The fix reuses the same signal `20260906000900_add_origin_to_sessions.
up.sql`'s own backfill already relied on for an identical problem:
`created_at == updated_at`. `update_session` advances `updated_at`
unconditionally on every write, with or without an `origin` field, so any
real edit — promoted or not — moves it away from `created_at`. The one
thing that had to change to make this reusable *live*, not just for a
one-time backfill: no machine write touches `updated_at` at all —
`provision_if_absent` no longer updates anything on conflict (`#345`), and
`refresh_if_untouched` deliberately leaves both stamps alone. If a machine
refresh bumped `updated_at` the way an early version of this fix did,
`created_at == updated_at` would break after the row's very first refresh,
for a proposal nobody had ever touched, defeating the check for the case it
exists to protect. With that
in place, `refresh_stale_proposal` gates on it directly:
`record.created_at != record.updated_at` stops the refresh outright,
leaving an edited-but-unpromoted proposal exactly as the person left it —
pinned by `an_edited_but_unpromoted_proposal_survives_a_refresh_pass_
untouched` in `nudge::waker`'s test module.

**A fourth real `chatgpt-codex-connector` finding on `#335`, P1, is about
the gap between that check and the write it gates.** `refresh_stale_
proposal` reads a row, decides — from that one read — whether refreshing
is safe, and only then writes. Between the read and the write sits a full
`engine.evaluate` call and an admission check: real time, and a real
window for a person's own `PATCH` or `DELETE` to land in. A decision made
from a read that may no longer be true is not a safety check, it is a
race — the exact shape of bug the "reads before it writes" section above
already names for `first_prepared`'s own staleness, now recurring one
layer up.

**`SessionRepository::refresh_if_untouched`** closes it the only way a
read-then-decide gap can be closed: by not trusting the read at write
time. It is a single `UPDATE ... WHERE id = ? AND subject_id = ? AND
origin = 'system' AND started_at IS NULL AND created_at = updated_at`,
scoped to the exact id `refresh_stale_proposal` already has, with
deliberately **no insert fallback**. That last part is what makes it safe
where `provision_if_absent` would not be: if the row was edited, started,
promoted, or deleted in the gap, the `WHERE` clause simply matches nothing
and zero rows are affected — an outcome `refresh_stale_proposal` treats as
"nothing to do," not an error. An insert fallback would have created a
second, orphaned proposal under a brand-new id while the `StudyAction`
`consider` already selected kept pointing at the old one, which is exactly
why this method exists separately from `provision_if_absent` rather than
reusing it. Since `#345` that method does not refresh at all — its
`ON CONFLICT` branch is `DO NOTHING`, because it runs before its pass can
hold a claim and so must never rewrite a row another pass may already have
sent a notification about. `refresh_if_untouched` is the one remaining
path that rewrites a proposal's content, and it is safe precisely because
`refresh_stale_proposal` calls it *after* winning that claim; its own
hazard is the read-then-write window above, which the `WHERE` clause
closes.

The checks inside `refresh_stale_proposal` itself (origin, `started_at`,
`created_at == updated_at`) are downgraded from a safety gate to a pure
optimisation by this fix: they decide whether it is worth doing the
catalogue read and `recommend()` call at all, and the atomic `UPDATE`'s
own `WHERE` clause is the only thing that actually has to be right at the
moment of the write.

**A fifth and sixth real `chatgpt-codex-connector` finding on `#335`, both
P1, both from the same root cause: `origin = 'system' AND started_at IS
NULL` is not the same predicate as "un-started," because a status change
can reach a `system` proposal without ever touching `started_at` at all.**
`SessionRepository::set_status_many` (`PATCH /sessions/status`) writes only
`status` and `updated_at` — a real, client-reachable path
(`updateStatusMany`, `paulgsc/some-ui`), not a hypothetical. It can move a
never-started `system` proposal straight to `active` or `completed` while
`started_at` stays `NULL`.

Two real consequences followed from that one gap, both in code this story
itself added:

- **Starvation, plus silent corruption, live.** Such a row still satisfied
  `idx_sessions_one_unstarted_system_proposal`'s predicate, so it
  permanently occupied the subject's one slot — `first_prepared` can never
  surface `active`/`completed` rows back out, since neither status is in
  its own tracked set, so no future proposal could ever be provisioned for
  that subject again. Worse, the row remained a live `ON CONFLICT` target,
  and at the time that branch was still a `DO UPDATE`, so the next
  provisioning pass would have silently overwritten the name, activities,
  and duration of a session that might be actively in progress or already
  finished. (`#345` has since made that branch a no-op outright, which
  removes the overwrite half of this independently — but the scoped
  predicate is still what keeps such a row from occupying the slot.)
- **Wrongful deletion, once.** The migration's own defensive dedup pass used
  the same broad predicate, so on a live database already holding such rows
  (from real use, not a bug in this migration) it would have deleted every
  `active`/`completed` row but the most recently touched one per subject —
  real history, not ignored proposals, and irreversibly, since the down
  migration only drops the index.

**The fix scopes both to the same three statuses `first_prepared` already
treats as "prepared"**: `idx_sessions_one_unstarted_system_proposal`'s own
predicate, `provision_if_absent`'s `ON CONFLICT` target, and the
migration's dedup `DELETE` all gained `AND status IN ('paused', 'scheduled',
'draft')`. An `active`/`completed` row reached through the `set_status_many`
anomaly now falls out of all three: it no longer counts toward the "at most
one" invariant (a fresh proposal can be provisioned alongside it), it is
never a conflict target (nothing ever overwrites it), and it survives the
migration's dedup untouched regardless of how many other such rows exist
for the same subject. Verified against a real scratch database seeded with
exactly this scenario (two genuine duplicate ignored proposals alongside
two real `active`/`completed` history rows sharing a subject) before
trusting the migration's own SQL — the duplicates collapsed to one, the
history rows both survived — and pinned in code by
`provision_if_absent_never_touches_or_is_blocked_by_a_set_status_many_
anomaly` in `crates/db/session/src/repository.rs`'s test module.

**A closing-review finding on `#335`, P1, turned up one more instance of
the exact race `refresh_if_untouched` already closes — in a different
call site.** `refresh_if_untouched` protects `refresh_stale_proposal`'s
own write; `provision_if_absent`'s `ON CONFLICT` branch (the
`NothingToSay` arm's write) had no equivalent guard. Its call site has no
fresh read of the conflicting row to check against — it discovers a
conflict only through the statement itself, against whatever a
*different* concurrent waker pass already inserted after this pass's own
`first_prepared` read found nothing. A person can edit that other pass's
freshly-inserted proposal (still `system`, `updated_at` moved, the same
pre-PRO1 gap) in the window before this delayed pass's write lands, and
the unconditional `DO UPDATE` would have silently overwritten it.

The fix stays inside the one `INSERT ... ON CONFLICT` statement:
`DO UPDATE SET ... WHERE sessions.created_at = sessions.updated_at`.
`SQLite` re-checks that condition atomically, in the same statement as
the conflict resolution itself — when it fails, the row is left
completely untouched and nothing is inserted either, verified directly
against a real database rather than trusted from documentation, since
this codebase had no prior use of this particular `SQLite` upsert clause
to point to as precedent. `provision_if_absent_never_overwrites_a_
conflicting_row_a_person_edited_since_it_was_inserted` reproduces the full
three-step interleaving directly: one pass's insert, a person's edit
landing on it, then a second pass's own conflicting write — and asserts
the edit survives untouched.

**`#345` has since superseded that fix with a stronger one.** The guarded
`DO UPDATE` above closed the person-edits-it case but not the one
`#342`'s review found: a machine write never moves `updated_at`, so a
*racing pass's* delayed write still satisfied `created_at = updated_at`
and could rewrite a row the winner had already sent a notification about.
The branch is now `DO NOTHING` outright, which closes both, and the test
named above still passes — its property is now structural rather than
conditional. See "Never stack proposals" above for the full argument.

**An eighth real `chatgpt-codex-connector` finding, on the second closing
review, is a race between two concurrent passes over the same subject —
not between a pass and a person.** Gating the refresh on `Verdict::
Intervene` (above) is necessary but was not sufficient: two `file_host`
instances can both read no presence for the same due subject and both
reach `Intervene`, and both had reached the refresh call under the
previous fix, since neither had lost anything yet at that point in
`consider` — the claim happens later, right before `actuate`. One pass
wins the claim and sends; the recipient can open that exact proposal
immediately. The other pass is still mid-flight — reading the session,
reading the catalogue, `materialize_provisioned_session`-ing — and its
own write, when it finally lands, still succeeds: `refresh_if_untouched`'s
guard is `created_at == updated_at`, which a machine refresh deliberately
never disturbs (that is what makes it survive *any* number of machine
refreshes), so the winner's own already-applied refresh does not stop the
loser's from landing too. The loser then reaches the claim, loses it
(`another pass claimed this subject first`), and sends nothing — but its
write already happened, silently rewriting the session out from under
someone who just opened it from the winner's notification, for an
intervention that was never sent.

The fix moves the call once more: from the `Verdict::Intervene` arm to
immediately after this pass's own successful claim, right before
`actuate`. A pass that loses the claim now returns before ever reaching
the refresh call, so the write this race depends on never happens on the
losing side at all — at most one pass per intervention ever refreshes,
and it is always the one that goes on to send. `a_pass_that_loses_the_
claim_never_refreshes_the_proposal_it_was_about_to_send` in `nudge::
waker`'s test module simulates the interleaving directly: it advances
`eligible_at` into the future between the two passes — exactly what the
winner's own claim would have done — so the pass under test still reaches
`Intervene` (`evaluate` never reads `eligible_at`) but then loses the
claim, and asserts the proposal's content is completely untouched
afterward.

### Nothing to say becomes a bug report (#285, RCM8)

`#279` (RCM2) hung provisioning off `Verdict::NothingToSay`, which is the set
of subjects `StudySelector::select` had no answer for: dominant deficit
`Momentum`, `Mastery`, or `Freshness`, with nothing to resume, review, or
announce. That is three of the four classes. The fourth never reaches that
verdict at all, because `#294` gave plain absence its one sessionless answer
— so the person this whole epic is named for, someone who has done nothing
but subscribe, got `GetStarted`'s invitation to go and find something rather
than the session RCM3/RCM4/RCM5 can now actually compose for them.

RCM8 changes the *condition*, not the machinery: provisioning now happens for
any **warranted** subject with nothing prepared, before selection is final.
`Verdict::Wait` is the one verdict that means "not warranted" — `evaluate`
returns it before selection is ever reached — and it is the only one
excluded. Everything else has already proven eligibility and refractory,
which is exactly the point at which a catalogue read is worth doing.
Provisioning ahead of a `Suppressed` verdict is deliberate and not new: RCM2's
arm already wrote the session first and checked admission after, so a subject
inside quiet hours ends the pass with a real proposal waiting for the next
admissible one. RCM8 only makes the `Presence` path behave like the other
three.

Selection and admission are then re-run against the session that now exists
(`nudge::waker::decide_with_a_proposal`) rather than the whole verdict being
recomputed. Warrant was settled before anything was written and nothing the
pass does afterwards can change it: `StudySignal::SessionProvisioned` carries
a delta of `0.0` precisely so that composing a proposal is not itself evidence
of engagement.

**What is left of `NothingToSay`.** Two facts now rule the verdict out of the
ordinary path. `evaluate` reaches it only when `select` returns `None`, which
it does only when `prepared_session` is `None`; and every warranted subject
with nothing prepared has just been through provisioning, which either handed
selection a `Some` — exhaustive over all four classes — or failed outright.
So the arm in `consider` is unreachable by construction and says so in a
comment: the only way there is a class added to `EngagementClass` without a
matching `StudySelector` arm, a build-time mistake rather than a state the
running system can drift into.

The one *reachable* silence left is a catalogue that can compose nothing —
unreadable, or with no timeable activity in it. A `Presence`-dominant subject
still gets `GetStarted` out of that, which is exactly the fallback role the
"Cold start" section above always said this story would either retire it into
or keep it for. Anyone else gets nothing, and that is now logged as the bug it
is, at `error!` and in a sentence about the catalogue: "there is nothing to
point them at" described an ordinary state of the world in RCM2's day, and a
log line describing a condition that no longer exists is worse than no log
line.

**`nudge_waker_nothing_to_say_total` is the alertable form of that sentence. A
non-zero value indicates a bug, not a quiet day.** It counts every pass that
ended with a subject the arithmetic said to interrupt and nothing to say to
them — an empty or unreadable catalogue, or the unreachable arm above. It is
deliberately its own series rather than another label on
`nudge_waker_verdicts_total`, whose every other value is an ordinary thing for
a healthy deployment to do; an alert can be written against this name without
depending on a label value surviving a refactor. A pass where an invitation
still went out does not count: that subject was told *something*, and the
failed proposal behind it is already in the logs and in
`verdict="storage_error"`/`"nothing_to_provision"`.

**The whole epic, as one test.** `a_subject_who_has_only_ever_subscribed_gets_
one_proposed_session_and_exactly_one_notification` in `nudge::waker`'s test
module is `#257`'s closing argument executed: a fresh database, one push
subscription, no sessions, no signals, no engagement rows; the clock advanced
past the solved eligibility instant; the waker run through `run_once` (not
`consider` — `due`'s own `WHERE eligible_at <= now` is the half of silence #1
that was broken, and a test that skipped discovery would assert the decision
without it). It then asserts every clause of the scenario: a session exists,
owned by that subject, `origin = 'system'`, its activities exactly what
`recommend()` picked and `provision()` floored, each block at
`max(activity floor, client minimum)`, exactly one notification accepted — over
a real socket, against a loopback push service that answers `201` — and a
second pass immediately after producing no second session and no second
notification. Every one of those clauses failed on `main` before this epic,
starting with the first.

### First contact: how a subject enters the gate at all

Until `#278`, nothing did. The waker's entire query is `WHERE eligible_at <=
now`, and `engagement_gate` rows were created in exactly one place —
`waker::observe`, called from `POST /signals` — whose only caller is the
client's session-mutation layer, fired when a session is *created*. The chain
was: no session ⇒ no signal ⇒ no gate row ⇒ never in `due` ⇒ never nudged.
Not late. Never. This is silence #1 in `#257`'s cold-start argument, and it
was silent for exactly the person who most needed the nudge: someone who had
just installed the app and done nothing else yet.

`POST /push/subscriptions` closes it. `waker::first_contact` runs on every
call, after the subscription itself is stored: it is a person explicitly
saying "you may interrupt me" — the strongest statement of intent this
deployment has — and, unlike a page load (too broad to count as consent) or a
dedicated "hello" route (a new endpoint restating what this one already
implies), it happens exactly once per device before any study behaviour
exists at all. Seeding lazily when the waker runs was never on the table: the
waker can only see rows that already exist, which is the circularity this
closes.

The row is written **full**, by the same `Charge::full`/`eligible_at`
arithmetic `observe` already falls back to for an unseen subject — so a
first-contact subject and a subject discovered through a stray signal land on
identical footing, and `eligible_at` comes out on the order of a week later
rather than five minutes. Idempotent by construction: the insert is guarded
by `engagement_gate`'s own primary key (`INSERT OR IGNORE`), not a
read-then-write, so a second device subscribing — or the same one
resubscribing after its keys rotate — cannot reset an already-drifting
subject back to full, and two devices racing to be the first one in cannot
both win.

The edge case worth naming: someone subscribes and then unsubscribes every
device before ever becoming due. They keep their gate row and have no way to
be reached. This degrades correctly rather than looping loudly — `Presence`
(fastest half-life, highest weight) stays the dominant deficit for a subject
who has never received a single signal, so `GetStarted` keeps firing when
they become due, and admission fails on `NotConsented` at `info!` rather than
falling through to `Verdict::NothingToSay`'s `warn!`.

Landing this closes **P0** (`#295`'s first deployable increment): a fresh
database, one subscription, no sessions, no signals, and the clock advanced
is now enough to produce one notification that opens the app.

## Consent is a precondition, not a preference

**The null case is silence.** There is no path through `push_repo` that creates a
subscription nobody agreed to: `upsert` takes a `Consent`, and an empty topic
list is honoured as "receives nothing" rather than read as "receives
everything". A topic list that cannot be parsed degrades to nothing, too —
consent that cannot be read is not consent.

`GET /api/v1/push/vapid-key` returns the topics on offer alongside the key, so
the permission modal renders the options the sender will actually honour rather
than a list maintained separately in the frontend.

Auth has not landed. `file_host::subject::SubjectId` is the seam: an extractor
that returns a singleton today and reads a validated token later, with every
table downstream already keyed by subject.

---

## Setting it up

### 1. Apply the migrations

**There is no in-app migration runner.** `build.rs` only declares
`rerun-if-changed=migrations`; nothing applies anything at startup. This is a
real step, not a detail, and it is the same step `capture_repo` and
`mood_event` already need.

The workspace owns one migration history in the repository-root `migrations/`
directory. Migrations are paired `<timestamp>_name.up.sql` / `.down.sql` files
and target the SQLite file named by `DATABASE_URL`. `sqlx-cli` expects the
forward migrations in a custom source to end in `.sql`, so stage the `.up.sql`
half of each pair before applying it:

```sh
cargo install sqlx-cli --no-default-features --features sqlite

# One database, one workspace migration history.
rm -rf /tmp/hopium-migrations && mkdir -p /tmp/hopium-migrations
find migrations -maxdepth 1 -name '*.up.sql' -type f | sort | while read -r f; do
  cp "$f" "/tmp/hopium-migrations/$(basename "$f" .up.sql).sql"
done

DATABASE_URL="sqlite:///path/to/hopium.db" \
  sqlx migrate run --source /tmp/hopium-migrations
```

This is also how `.github/actions/prepare-sqlx/action.yml` prepares image builds.
The test and lint workflows apply the same root migration history before
compiling SQLx queries. Keep all three in step if the migration layout or
staging convention changes.

#### Building without a database

`sqlx::query!` verifies SQL at compile time, so `cargo check` needs either a
`DATABASE_URL` pointing at a migrated database, or the offline cache:

```sh
DATABASE_URL="sqlite:///path/to/hopium.db" cargo sqlx prepare --workspace
```

`.sqlx/` is in `.gitignore` on purpose: the cache is a build artifact of a
schema the migrations already define, and committing it means a second copy of
the schema to keep in step. CI therefore builds it, the same way you just did —
`test.yml`'s `rust_ci` job creates a database, runs the workspace migrations
into it, and runs `cargo sqlx prepare` before `cargo check`. That job had been
running a bare `cargo check` with no database and failing before it compiled
anything; the setup steps are what make it a real check.

If you change how migrations are applied, `test.yml` and `lint.yml` are the two
places that have to know.

The tables added by this feature:

- `push_subscriptions` — one row per browser, with its owner and its consent.
- `engagement_charge` — `(subject, class, level, as_of)`. Undecayed; decay is a
  function of the stamp.
- `engagement_gate` — the solved `eligible_at`, and the index the waker reads.
- `intervention_log` — what actually went out, and whether it reached a push
  service.
- `sessions` — the server's copy of `SessionRecord`.
- `activities` — the server's copy of `ActivityDefinition` (#269). Deliberately
  the odd one out: every table above carries a `subject_id` as of #259, and
  this one does not, because an activity is catalogue-wide — a fact about what
  exists to play, not about who has played it. Per-subject state (played,
  dismissed) is #258's table, not a column here.

  Seeded with the four activities `paulgsc/some-ui@packages/activity-catalog`
  bundles (#270, `20260823000700_seed_activities.up.sql`), transcribed field
  for field. The parity test guarding that transcription lives in
  `crates/db/activity/tests/catalog_parity.rs`; its own module doc records
  which of #270's three proposed mechanisms this took and why the other two
  were deferred. What this table deliberately has no column for —
  `toSceneProps` — and what a server-composed session writes instead, is
  [its own decision below](#the-server-writes-data-the-client-writes-behaviour-272)
  (#272).

### 2. Generate a VAPID keypair

```sh
npx web-push generate-vapid-keys
```

Or, with only OpenSSL:

```sh
openssl ecparam -name prime256v1 -genkey -noout -out vapid_private.pem
openssl ec -in vapid_private.pem -outform DER | tail -c +8 | head -c 32 \
  | base64 | tr -d '=' | tr '/+' '_-'          # VAPID_PRIVATE_KEY
openssl ec -in vapid_private.pem -pubout -outform DER | tail -c 65 \
  | base64 | tr -d '=' | tr '/+' '_-'          # VAPID_PUBLIC_KEY
```

Put the two base64url strings in the environment. `.gitignore` already covers
`*.pem` and `.env`; keep the PEM out of the repository.

#### Rotation is a migration, not a config change

**The public key is baked into every subscription made with it.** Rotating it
does not re-key those subscriptions — it silently invalidates all of them. The
failure looks like "notifications just stopped", with a `403` buried in a log
that nobody is reading, because a feature that is supposed to be quiet most of
the time is exactly the kind that can be broken for a week unnoticed.

Recovery is every browser visiting the site and subscribing again. There is no
server-side fix. So:

- `file_host` derives the public key from the private one at startup and
  **refuses to boot** if the configured pair does not match. That mismatch is
  the single longest-fuse failure in this feature, and it is worth a boot
  failure to catch.
- `GET /api/v1/push/vapid-key` exists so the frontend never hardcodes the key.
  A key change should not require a frontend redeploy, and an
  `applicationServerKey` mismatch is invisible until a send fails.

There is deliberately no rotation *mechanism*. Understanding the cost is in
scope; automating it is not.

### 3. Configure the rest

See `example.env`. The two that most reward attention:

- **`NUDGE_TIMEZONE`.** Containers are UTC unless told otherwise. A UTC day
  boundary files an 18:00 local session under *tomorrow* in a UTC-07:00 zone —
  so "have I studied today" answers no, and you get nudged for a day you already
  did. The same offset makes quiet hours of 22:00–08:00 silence 15:00–01:00
  local and permit 03:00. Leaving it unset is allowed, and is logged loudly at
  startup rather than assumed.
- **`NUDGE_ENABLED`.** With this off, the `/push` routes still work and
  `POST /api/v1/push/test` still sends; only the schedule is idle. With it on
  and no usable VAPID pair, startup fails.

### 4. Serve the app over HTTPS

Service workers, `Notification`, and `PushManager` are all gated on a **secure
context**. `http://localhost` and `http://127.0.0.1` qualify by explicit
exception; `http://nixos.local` and `http://192.168.x.x` do **not**.

The symptom is confusing: the APIs are simply absent from `window`, so what you
see is "the toggle isn't there", not an error. On a LAN-only setup this is the
blocker that stops the whole feature.

The client already handles its side — `apps/www/vite.config.ts` in
`paulgsc/some-ui` picks up `certs/nixos.local+3.pem` when present, and the
settings section renders an explanation instead of the toggle when
`nudgesSupported()` is false.

#### The certificate story

- **Which cert.** A locally-issued cert for `nixos.local`, generated with
  [`mkcert`](https://github.com/FiloSottile/mkcert):
  `mkcert nixos.local localhost 127.0.0.1 ::1`, which writes
  `nixos.local+3.pem` and `nixos.local+3-key.pem`.
- **Who trusts it.** `mkcert -install` puts the local CA into the system trust
  store, and separately into Firefox's (which keeps its own). Every browser that
  is going to subscribe needs the CA trusted, or the page is not a secure
  context and there is nothing to subscribe with.
- **When it expires.** `mkcert` leaf certificates are good for a little over two
  years; the local CA for ten. Expiry presents as the app failing to load over
  HTTPS, and then — because there is no secure context — as the nudge toggle
  disappearing. The remedy is to re-run the `mkcert` command above and restart
  the dev server. If the *CA* expired, `mkcert -uninstall && mkcert -install`
  first, and every browser has to trust the new one.

Write the date somewhere you will see it. A certificate expiry that nobody
expected is indistinguishable, from the outside, from this feature being broken.

---

## The HTTP surface

All paths are under `/api/v1`.

### Push

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/push/vapid-key` | The `applicationServerKey` and the topics on offer |
| `POST` | `/push/subscriptions` | The browser's `PushSubscription` **plus the topics they agreed to**, and first contact — see below |
| `DELETE` | `/push/subscriptions` | Withdrawing consent, idempotent, body `{ endpoint }` |
| `POST` | `/push/test` | Send now, without waiting for a day to pass |

### Signals

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/signals` | A domain event: `session-started`, `session-abandoned`, `scored-below-target`, `curriculum-updated`, … |

**This is where a charge is updated.** A signal folds into the subject's
existing charge and the crossing instant is re-solved on the spot; the
response returns the new `eligible_at`, which makes the arithmetic observable
without waiting for a notification. A cron-driven design has no endpoint like
this, because nothing needs to tell it anything — and that convenience is
exactly what costs it the ability to answer *why*.

It is not, since `#278`, the only place a subject *begins* to exist:
[first contact](#first-contact-how-a-subject-enters-the-gate-at-all)
(`POST /push/subscriptions`) writes the initial gate row, seeded full, before
any signal has ever arrived.

### Outcomes

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/outcomes` | How one activity block of a session went — `{ sessionId, blockIndex, activityId, startedAt, endedAt, plannedMs, elapsedMs, outcome, score? }` (#287) |

An applet that just graded someone says so here, and the server — not the
applet — decides what that means for the engine. The block is written to
`activity_outcome` (one row per block; see its migration for the grain), and a
signal is **derived** from it by `study_domain::signal_for_block`, beside the
calibration numbers: a completed block scored below `SCORE_TARGET` (0.7) is
`ScoredBelowTarget`, `scored-below-target`'s first real producer. Everything
else derives nothing — finishing is `SessionCompleted`'s to credit, and an
abandoned block is already `SessionAbandoned`, which counting twice would drain
momentum twice.

Replays are safe: `(sessionId, blockIndex)` is the key, only the first report
of a block writes a row, and only that one folds a signal. An identical retry
answers `replayed: true`; a contradictory one is a `409`. Invalid input — a
block index outside the session, the wrong activity for that block, more
elapsed time than the whole session, a score outside `[0, 1]` or on a block
that did not complete — is a `422` naming the field, never clamped. Another
subject's session is a `404`.

`score` is optional and `null` means *not assessed*, which is not the same as
scoring zero. Send it only for an assessment of the block as a whole — a
LeetType round's single selection is not one (#329): it would make a wrong tap
raise a notification.

### Curriculum

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/curriculum/manifest` | `{ version, topiks: TopikMetadata[] }` — the manifest `@some-ui/topik` reads, from the `curriculum` table (#276) |
| `GET` | `/curriculum/:key` | One lesson file, verbatim, or a JSON `404` |

The same two files `apps/www` fetches from `/topiks/` today, with the one
difference the route exists for: a lesson that does not exist is a real `404`
with an error body, where a static server behind `try_files … /index.html`
answers `200` with a page. An empty corpus is a valid, empty manifest at `200`.
Both answer `If-None-Match` with `304`: a lesson's `ETag` is its `content_hash`
(#274), and the manifest's is `content_hash` over its listing — which is also
its `version` — so one notion of "changed" serves the importer, the cache, and
#277. The manifest is bounded (`MANIFEST_CEILING`) and refused, never
truncated, above it. A lesson keyed `manifest` would be shadowed by the listing
route; the importer's corpus has none.

### Subject stats

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/subjects/me/stats` | Per-activity plays, completion and abandonment rates, mean assessed score, last play — plus `history` in the client's `RankingSignals.history` shape (#289) |

One row per activity the subject has an outcome for, from `activity_outcome`,
in one grouped query over `idx_activity_outcome_subject_activity` — bounded by
the catalogue (`outcome_repo::STATS_CEILING`), not by how much someone has
played. `history` is one `ActivityPlay { activityId, at }` per activity at its
most recent play (ms since the epoch); `activities[].plays` carries the count
the client's frequency axis would otherwise get by counting entries. Skipped
blocks are not plays, and a `null` score is excluded from the mean rather than
averaged in as zero.

#### Outcome stats and the flag that gates them

`RECOMMENDER_USES_OUTCOMES` (default **off**) decides whether a provisioned
session is ranked with these stats (`RankingInputs::from_stats`): an assessed
mean becomes `Completed { score }`, abandoned-and-never-completed becomes
`Abandoned`, completed-but-unassessed becomes `Unassessed` (played, but no
evidence it landed), and an activity only ever skipped stays unplayed. Off, the
recommender sees every activity as never played — the cold-start ranking it
has used since #280. A subject with no outcomes gets the identical proposal
either way, and a subject's stats are read once per decision, so the seeded
shuffle stays deterministic.

**Turning it on is a decision, not a deploy.** What would justify it, looked
at on real data rather than on whoever tested it:

- enough outcomes to rank on — most active subjects with several assessed
  blocks across more than one activity;
- abandonment rates and mean scores that actually *separate* activities — if
  every activity abandons at the same rate, axis 2 is noise;
- proposals, recomputed offline for real subjects with the flag on, that a
  person looking at them would call sensible — a poorly-scored activity
  offered again, a mastered one offered less.

### Presence

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/presence/lease` | `{ context_key }` — "I am looking at this, right now" |

The client posts here on a visibility/route transition and again on a sparse
renewal (roughly every 45s) while visible — not a poll loop. `context_key` is
whatever `StudyAction::session_id()` would return for the thing being looked
at, almost always a session id: `nudge::presence` matches leases against that
same value, so a client and the waker agree on what a "context" is without
either one importing the other's types. See
[Presence is a lease, not a veto](#presence-is-a-lease-not-a-veto) for why this
replaced a WebSocket connection count.

### Sessions

One route per `SessionsRepository` method in
`apps/www/src/lib/tenant/sessions-repository.ts`, matched one-to-one so that the
client change is a swap rather than a rewrite:

| Client | Server |
|---|---|
| `list()` | `GET /sessions` |
| `get(id)` | `GET /sessions/:id` |
| `create(input)` | `POST /sessions` |
| `update(id, patch)` | `PATCH /sessions/:id` |
| `remove(id)` | `DELETE /sessions/:id` |
| `removeMany(ids)` | `DELETE /sessions` — `{ ids }` |
| `updateStatusMany(ids, s)` | `PATCH /sessions/status` — `{ ids, status }` |
| `duplicate(id)` | `POST /sessions/:id/duplicate` |

Two behaviours moved server-side with the data, because leaving them on the
client would let the two diverge silently:

- **Id generation.** `session-<uuid>`, as `generateId("session")` produced.
- **`totalDurationOf(scenes)`** — `max(start_time + duration)`. If the server
  stores `total_duration_ms` but lets the client compute it, a client that
  forgets stores a zero, and the nudge cheerfully offers you a "~1 min" session.

**`sessions` has an owner now.** #259 added `subject_id`, backfilled to
`SINGLETON_SUBJECT` for every row that existed before the migration, and
rebuilt `idx_sessions_status` as `(subject_id, status, updated_at DESC)` for
the per-subject scan #260 (SUB2) then added: every non-admin method on
`SessionRepository` (`list`, `get`, `upsert`, `delete`, `delete_many`,
`set_status_many`, `touched_between`) now takes a `subject_id` and is scoped
by it — `get` returns `None` for a foreign id rather than the row (an id is
not a capability), `delete`/`delete_many` treat a foreign id as a no-op, and
`upsert` refuses outright, as `SessionRepoError::SubjectMismatch`, to move an
existing row to a different subject. The route layer does not extract a real
subject yet — `handlers/db/session.rs` passes `SINGLETON_SUBJECT` explicitly,
same as every other unauthenticated caller — that thread-through is #261
(SUB3), whose own acceptance criterion (`grep -rn "SINGLETON_SUBJECT"` outside
`subject.rs` finds nothing) is what retires those call sites.

### Activities

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/activities` | The full catalogue, bounded (`activity_repo::CATALOG_CEILING`) |
| `GET` | `/activities/:id` | One activity, or `404` |

Read-only. #271's own out-of-scope note is explicit: the catalogue is seeded
and migrated, not written through this surface, so there is no `POST
/activities`.

No `SubjectId` scoping either, unlike Sessions above: an activity is
catalogue-wide, not owned by whoever plays it — `activity_repo`'s own crate
doc makes the same point about per-subject state (played, dismissed) living
in a different table, not a column here.

**`ETag`, and the one thing it has to agree with #273 about.** Both routes
answer `If-None-Match` with `304` and no body. On `GET /activities` the tag
is `ActivityRepository::fingerprint()` — a hash over every row's `(id,
version)`, so a publish, an edit, or a removal all invalidate it, not just an
increasing `version` somewhere in the set. On `GET /activities/:id` the tag
is `id@version`, scoped to the one row that response actually returned,
rather than the whole-catalogue fingerprint — an unrelated activity's edit
should not invalidate a client's cached copy of this one. `fingerprint()` is
*not* what #273 (CAT5)'s `CurriculumUpdated` producer reads, although this
section once said it would: a single hash says *that* the catalogue changed
but not *which* entry, and the producer has to know which `(id, version)` is
new to announce it. It compares `activities` against its own
`curriculum_publication` log instead — see "New material is one epoch, not a
fan-out" below.

**The bound is a refusal, not a truncation.** `GET /activities` counts the
table before querying it; over `CATALOG_CEILING` rows and the whole request
is refused (`FileHostError::MaxRecordLimitExceeded`) rather than answered
with a silently short prefix. The same bounded-or-refused-never-silently-partial
invariant `#253` argues for elsewhere applies to this new surface too.

### Trust model, stated plainly

These routes carry **no authentication** beyond the CORS origin allowlist.
Anyone who can reach the LAN can register a push subscription — and would then
receive this person's study reminders — or read and write sessions. That is the
same trust model as the rest of `file_host`. It is written down here so it stays
a conscious acceptance rather than an oversight.

---

## The decisions worth knowing

### `201` does not mean delivered

A push service returns `201 Created` once it has **accepted** a message. Not
delivered, not displayed, and — the trap — not decryptable: a payload encrypted
with the wrong keys returns `201` and is then silently discarded by the browser,
because the push service never had the key material to notice.

Nothing in `nudge::sender` logs a `201` as though it meant delivery, and the
only real proof is a human seeing a notification. Plan the first test that way.

The failures are where the information is:

| Status | Meaning | What happens |
|---|---|---|
| `201` | Accepted for delivery | Recorded; proves nothing |
| `400` | Malformed request or bad VAPID JWT | Logged loudly; it is a bug here |
| `403` | VAPID key mismatch | The key was rotated — see above |
| `404` / `410` | Subscription is dead | The row is deleted |
| `413` | Payload too large | Refused before sending too |
| `429` | Rate limited | `Retry-After` surfaced; no retry queue |

A `403` deliberately does **not** prune. It means *this server* signed with the
wrong key, and pruning on it would delete every subscription over one bad
environment variable.

### Presence is a lease, not a veto

Presence used to be a WebSocket connection count, and that was wrong two
independent ways, discovered together rather than one at a time:

1. **The client this feature is for never opened one.** `document.
   visibilityState` — the signal the client actually has — was never wired to
   the server at all, so the WS-connected check almost never fired for a real
   user. Silence that looked like a working, quiet feature was actually a
   feature whose main signal was structurally dead.
2. **This deployment's own synthetic monitoring produced false positives.**
   `infra/blackbox.yml`'s WS health-check prober completes a real upgrade
   against `/ws` every 15 seconds and hangs up without sending a frame. The
   old connection count did not — and structurally could not — tell that
   apart from a person, so the app's own liveness probe could suppress a
   real notification.

Both failures trace to the same root cause: a WebSocket connection answers "is
a socket open," and presence needs to answer "is someone looking at *this*."
Those are different questions, and no amount of freshness-window tuning on the
first one produces the second — freshness filters on *recency*, and a
brand-new probe connection is, by construction, always recent.

**The fix is a lease, not a better connection count.** The client writes
`{ subject_id, context_key, observed_at }` (`POST /presence/lease`) on
meaningful transitions — the tab becoming visible, the route changing — plus a
sparse renewal (~45s) while visible. `nudge::presence::observe` reads every
lease a subject holds once per `waker::consider` pass and hands the snapshot
to `StudyConstraints::admit`, which asks a narrower question than the old
design ever could: *is there a fresh lease on exactly the context this
notification is about*, not *is this person on the site at all*.
`context_key` is `StudyAction::session_id()` — every variant but
`GetStarted` already carries one — so a lease on some other session can never
suppress a notification about this one, and `GetStarted` (no session to point
at) is never suppressible by presence at all.

Freshness is derived lazily rather than maintained: `now - observed_at < ttl`
(`NUDGE_PRESENCE_LEASE_TTL_SECONDS`, default 75s), computed at the moment
`admit` asks. A lease six months stale reads exactly like no lease — there is
no expiry job, and none is needed for correctness.

The asymmetry is deliberate, not a compromise: uncertain or missing presence
**sends**; only a fresh, context-matching lease **suppresses**. A storage
failure while reading leases is treated as "no lease" rather than propagated,
for the same reason a connection actor that could not answer used to count as
absent — presence must never be able to fail *closed*. The failure mode this
guards against is the same one that motivated the original design: someone
who keeps the dashboard open and forgets about it must never have their
notifications silenced forever by a signal that broke.

The one invariant that keeps this from regrowing into a realtime subsystem:
**presence must never cause work; it can only modify work that was already
about to happen.** Nothing scans every subject's leases on a timer — the only
read happens inside `waker::consider`'s existing per-subject admission check,
at the moment a candidate notification already exists and is about to be
sent.

### One intervention at a time, across restarts

The claim is taken **before** the send, in a single conditional `UPDATE ... WHERE
eligible_at <= now` that also moves `eligible_at` forward. Two waker passes — or
one pass and a just-restarted process — cannot both conclude a subject is due. A
crash between claiming and sending therefore costs that intervention rather than
duplicating it, which is the right way round for a feature whose whole value is
not being annoying. If every device fails, the claim is released — unless a
delivery **timed out** (#264, SLI3). A timeout is crash-shaped: it cannot say
whether the push service took the message, and a provider that received the
request and never answered may still deliver it. So it lands on the same side
of the line a crash does — the claim is kept, the intervention is spent, and
`intervention_log.actuated_at` stays `NULL` because nothing confirmed
acceptance. The timed-out device is recorded as a failure (never a prune).

**Every waker write to a charge is version-checked** against the charge that
pass read (#287/#360). Signals fold in under a `BEGIN IMMEDIATE` write lock
(`EngagementRepository::fold`), so concurrent signals never lose each other; and
a waker verdict — a `Wait`/`Suppressed` save, or the claim with its recharge —
is written only if no signal landed since the pass read the charge. If one did,
the claim is refused and nothing chosen from the stale deficits is sent; the
subject is reconsidered next pass from where the signal left them.

Two further brakes: an intervention **recharges** the classes it addresses, so
the next pass finds nothing to do; and `REFRACTORY` is a hard floor whatever the
arithmetic says.

### A read reachable from the waker declares its own bound

**There is no caller to paginate it.** `GET /sessions` can leave `list()`
unpaginated because the client is the one caller, and the client already
paginates the result in memory. `nudge::waker::consider` broke that: nothing
sits above the waker to page through what a query on this path hands back —
the tick fires, the pass runs, and the read happens in full. A query on this
path that assumes some caller will bound it is assuming a caller that does
not exist.

So the invariant is stated the other way round: **a read reachable from the
waker declares its own bound**, in the query itself (a `LIMIT`, an indexed
`WHERE`, or a narrower question than "everything") rather than in a caller
that isn't there to enforce one. `list()` narrows *which subject's rows* it
can see (#260/SUB2 — a query can no longer return another subject's sessions
at all) but does not bound *how many*; `#262` (SLI1) measured that gap, a
characterisation test in `nudge::waker`'s test module pinning down
`list()`'s `O(BATCH × sessions this subject owns)` cost. `#263` (SLI2) closes
it: the waker no longer calls `list()` at all. `SessionRepository::first_prepared(subject)`
replaces the list-then-find with a purpose-built, `LIMIT 1` query backed by
`idx_sessions_status` (`(subject_id, status, updated_at DESC)`) — a `paused`
session outranks a `scheduled` one, which outranks an untouched `draft`, with
`updated_at DESC` breaking ties within one status (see `first_prepared`'s own
doc comment for why). #262's characterisation test now asserts this bound
directly: at most one row read per due subject, independent of how many
sessions it owns.

### New material is one epoch, not a fan-out (#273, CAT5)

`CurriculumUpdated` — freshness's −35, the case a pure activity model cannot
express — has a producer, and it costs O(1) in subjects (see "Scaling
invariants" above).

**Publishing is one append.** Each waker pass records every catalogue `(id,
version)` it has not seen before in `curriculum_publication`: a new activity or
a version bump is new material. That table is an append-only log, and its
`MAX(id)` is the **epoch**. Detection reads the catalogue (bounded, and refused
over its ceiling), never the subjects.

**Each subject carries one watermark**, `engagement_gate.curriculum_epoch`: the
newest epoch already folded into their charge. `due` returns subjects whose
`eligible_at` has passed *or* whose watermark is behind the epoch, from two
indexes. For a behind subject the waker folds **one** `CurriculumUpdated` and
advances the watermark to the epoch in the same `BEGIN IMMEDIATE` transaction
(`EngagementRepository::catch_up`), then considers them as usual if the drain
made them eligible.

- **Idempotent by construction.** The watermark only moves forward, and only
  with the drain, so a re-run pass, a crash, or two racing passes cannot drain
  anyone twice — there is no claim table to get right.
- **One drain per catch-up.** A subject who missed three publications is
  drained once: the signal means "there is new material", and a drain per
  publication would let a burst of catalogue edits empty someone's freshness in
  one pass.
- **Bounded by `BATCH` and the pass deadline**, like every other due subject.
  More than a batch behind is simply more than a batch due; later passes finish
  the rest.

**Who it drains** is `study_domain::CURRICULUM_AUDIENCE`, beside the
calibration numbers: **every subject the nudge knew before the publication**. A
gate row is stamped with the current epoch when it is created — first contact
or a first signal — so someone who arrives after a publication is never behind
it; the migration stamps every existing row with the baseline epoch, so
deploying this drains nobody. Nothing a subject later edits or deletes changes
whether they were known. The accepted cost: someone who subscribed but never
studied is drained too.

**Lessons append to the same log (#277, CUR4)** — a second producer, not a
second mechanism. A lesson `(key, version)` not seen before is a publication
with `source = 'curriculum'`, and its version only moves when the file's bytes
do (#275), so re-importing unchanged content or renaming a lesson publishes
nothing; the importer's first import writes `baseline` rows, which are seen but
are not an epoch. A lesson's audience is narrower,
`study_domain::LESSON_AUDIENCE`: subjects who had **played its activity** (a
completed or abandoned block in `activity_outcome`) by the time it was
detected. That rule is applied per subject **at catch-up**, not as an audience
query at publish time: `PublicationRepository::relevant_since` walks the
publications a subject missed, newest first, and stops at the first that
applies to them — O(publications missed), a point lookup each. If none does,
their watermark moves up with no drain. A lesson at a level far from someone's
target would be closer still to noise, but `targetTopikLevel` lives in the
client's profile and this server has never seen it; scoping by level waits on
profile targets moving server-side, and is named as that dependency rather than
guessed at.

### Importing lesson content (#275, CUR2)

Lessons live in the `curriculum` table (#274) — one row per lesson, the lesson
file itself stored verbatim — rather than in `paulgsc/some-ui`'s
`public/topiks` directory. An operator puts them there, offline:

```sh
DATABASE_URL=sqlite:///path/to/file_host.db \
  cargo run -q --bin import-curriculum -- path/to/public/topiks --dry-run
# then, if the report looks right, without --dry-run
```

Run it when the corpus changes — a lesson added or edited in `some-ui` — and
once per environment to bring an existing corpus across. It is safe to run
again and again: a lesson whose file bytes are unchanged (`content_hash`,
SHA-256 over the exact bytes) is not written and does not look new; changed
bytes are a version bump with a new `published_at`; a manifest rename alone is
written without either. A malformed or missing lesson fails alone and is named
in the report — except on the first import, which is all or nothing (below);
the exit code is `1` if anything failed, `2` if the run could not start. It never deletes a lesson missing from the directory.

**The first import is a baseline.** Into an empty table, what is imported is
what the app has served all along, so every lesson is written to
`curriculum_publication` as a `baseline` row: *seen*, so #277 never mistakes it
for new, but not an epoch — the epoch is the newest non-baseline row — so nobody
falls behind it and no watermark is touched. It is all or nothing: if any lesson
fails, nothing is written, so the re-run after fixing it is still the first
import — a partly written one would leave the repaired lessons to be announced
as new. Every later import's new or changed lessons are what #277 announces.

Nothing in the server's startup or request path runs or waits on this.

### A pass is bounded, not just a request (#264, SLI3)

`TimeoutLayer` bounds inbound HTTP. The waker is a spawned loop with no
`tower` layer above it, and it is serial on purpose — a burst that would
notify a whole userbase at once is worth rate-limiting into. Before #264,
serial also meant *unbounded*: one push provider that stopped answering held
every later subject in the batch for as long as it stayed silent. Two bounds
now, at the two levels where running out means something different:

- **Per delivery — `PUSH_DELIVERY_TIMEOUT_MS` (default 10s).** A provider that
  has not answered is a failure against that device (above), and the loop moves
  on to the next device and the next subject.
- **Per pass — `WAKER_PASS_DEADLINE_MS` (default 2 min, inside the 5-minute
  interval).** Checked between subjects *and* between one subject's devices:
  each delivery's timeout is the lesser of the delivery timeout and what the
  pass has left, and a device reached after the deadline is not tried (a
  subject's device list is unbounded, so the per-delivery bound alone would let
  one subject hold a pass for `devices × timeout`). `consider` itself is never
  cancelled part-way — it claims before it sends, and cancelling it would put a
  crash-shaped event on an arbitrary side of that line — so what remains past
  the deadline is storage work `busy_timeout` bounds. Subjects the pass did not
  reach are left exactly as `due` found them and are picked up next pass — the
  same property `BATCH` relies on.

The NUDGE row's *Waker Pass Duration* panel draws the last pass against the
configured deadline; `nudge_waker_pass_deadline_exceeded_total` counts passes
that ran out, and its healthy value is zero.

### History has a horizon (#265, SLI4)

`intervention_log` exists to answer "why did I get that notification?" — and
**that answer expires after 90 days**
(`engagement_repo::INTERVENTION_LOG_RETENTION_DAYS`). Confusion about a nudge
arrives late, so the horizon is long; at one row per intervention it costs
almost nothing to keep. Past it, the row is gone and the question can no longer
be answered from this table.

The rule, stated so the next history table can cite it rather than re-derive
it — #286's `activity_outcome` is the first that does, with its own horizon of
**one year** (`outcome_repo::ACTIVITY_OUTCOME_RETENTION_DAYS`; a person's study
history answers "have they played this" across a summer away, which ninety days
would not):

1. **Time-based, on the row's own event timestamp** (`decided_at` here). Not
   count-based: "the last N" answers *how many*, and the question is *when*.
2. **An index on that timestamp** (`idx_intervention_log_decided_at`), so the
   sweep is a range read — one index probe on a day with nothing to delete.
3. **A bounded delete** (`RETENTION_SWEEP_LIMIT`, 500 rows, oldest first), so a
   first run against a table that has grown since launch drains over several
   passes instead of becoming the long pole in one.
4. **Run from the waker's pass**, after the subjects it had to handle and only
   if the pass deadline (above) has not run out — not from a second scheduled
   task. The waker is already a bounded, cancellable loop; the sweep inherits
   both. A sweep failure is logged and does not fail the pass.

Rows with `actuated_at IS NULL` — claimed but never confirmed: a crash between
claim and send, or a timed-out delivery — are **not exempt**. That window
matters for minutes; nothing reads those rows to recover anything, because the
claim is deliberately final. Exempting them would make them the only rows with
no horizon at all.

`engagement_charge` deliberately has **no** time-based horizon. It is bounded by
construction (one row per subject × class), and its rows are current state, not
history: deleting a long-silent subject's charge would reset them to *full* —
exactly the person the charge exists to notice. A subject whose account is gone
leaves four rows behind; removing them belongs to whatever deletes the account,
which does not exist yet. `sessions` retention is out of scope for the same
reason in the other direction: a person's sessions are their data, not the
system's history.

### One policy, one language

An earlier draft kept a JSON fixture so a TypeScript copy of the policy and a
Rust copy could be checked against each other. That is gone. The typestate is
Rust: signal classes, weights, half-lives, and the decision are all types the
compiler checks, and anything user-specific is an instantiation persisted per
subject. There is no second implementation to keep honest, and a fixture file
would only have been a weaker restatement of the enums.

The client keeps `decideNudge` for the GitHub Pages build, which has no backend
at all — but that is a *fallback*, not a mirror, and `paulgsc/some-ui#924` is
where the two are prevented from both firing.

### The server writes data; the client writes behaviour (#272)

Every `ActivityDefinition` (`packages/activity-catalog/src/lib/types.ts`,
`paulgsc/some-ui`) carries `toSceneProps`, a closure mapping a friendly config
onto a scene's `props`. There is no JSON encoding of a function, so a
catalogue this server serves arrives without one — and a session this server
*composes* (`#279`–`#285`) hits the same wall from the other side: it cannot
write scene `props` it has no way to compute.

Two ways of closing that gap were considered and are recorded here as
**rejected**, so neither gets re-proposed:

- **Ship the source and `eval` it.** Remote code execution as a product
  feature. Not an option, for any reason.
- **Ship a template language and interpret it here.** This server would need
  a mini-interpreter, and the boundary the client's own catalogue already
  draws would sit underneath it unenforced. `interview`'s entry is explicit
  about why it passes an identifier rather than resolved content: *"importing
  anything from `@some-ui/interview` for a value puts that package in this
  app's eager bundle — undoing the lazy import the content registry exists
  for."* A server-side renderer has no way to know that, and would happily
  let someone violate it.

**What this server actually writes** is `activities: [{ activityId, config
}]` — already what the client's `SessionActivity`
(`packages/activity-catalog/src/lib/to-scene-config/index.ts`) is — and
nothing else. The client's existing `toSceneConfig`/`sequenceScenes` do the
rest, the same way they already do for a Basic-composer session where the
user made zero arrangement decisions: a server-composed session is not a new
code path on the client, it is the existing one fed data from a different
source. `paulgsc/some-ui#1036`'s ACT2 is where the closures move into a
resolver keyed on `registryKey` instead of living on each definition, but
that move is independent of this conclusion — the server was never going to
call them either way.

**The catalogue schema already honours this.** `ActivityRecord`
(`crates/db/activity/src/model.rs`) has no `toSceneProps` field at all —
`fields`, `default_config`, and `audio` are the only JSON-blob columns, and
each is opaque *data* this crate persists without interpreting, never a
column that only makes sense as executable code. Nothing about landing this
story required a schema change.

**The `scenes` consequence, faced honestly.** If the server does not write
`props`, it cannot write playable `scenes` either — and `scenes` is `NOT
NULL`. A session this server provisions therefore writes `scenes` as `'[]'`:
a real, valid empty JSON array, not a guess dressed up as a placeholder and
not the string `'null'`. That is a session state nothing before this story
produced — `activities` non-empty, `scenes` empty — and it is deliberate, not
an oversight: turning `activities` into playable `scenes` is the client's
job, using the same `sequenceScenes` pipeline a Basic-composer session
already runs, at whatever point the client chooses to materialise them
(closing that loop is `paulgsc/some-ui#1038`'s PRO1, not this story).
**RCM5 (#282)** is where a provisioned row actually gets written, and its own
"the hard one" section already arrives at the same three options this
paragraph does; #282 is expected to cite this decision rather than re-argue
it. The combination consistent with the conclusion above is #282's option 1
(`scenes: []`) for what the server writes, materialised later by #282's
option 2 (the client computing real scenes and writing them back) rather
than option 3's half-measure of structurally-complete scenes with empty
`props` — a shape that would look valid and would not be.

The round trip this implies — a catalogue row's `default_config` is
sufficient, as-is, to become a `SessionActivity.config` the client's
`toSceneConfig` accepts — is checked on both sides of the boundary:
`crates/db/activity/tests/session_activity_shape.rs` here, asserting every
seeded `default_config` is shaped like `ActivityConfigValues` (a flat object
of strings and numbers, nothing nested); `paulgsc/some-ui`'s
`packages/activity-catalog/src/lib/to-scene-config/session-activity-round-trip.test.ts`,
asserting a `SessionActivity` built the same way survives an actual JSON
round trip into a playable scene.

---

## Known unknowns

What this feature cannot tell you, written down so the next piece of work starts
from a stated boundary rather than rediscovering it.

**Connected is not focused.** The server sees WebSocket connections, not tab
visibility. A dashboard open on a second monitor and ignored for six hours looks
exactly like one being read. The freshness bound narrows this but does not close
it; only a client-side visibility report would.

**There is no delivery confirmation.** The Web Push protocol does not offer
one. `last_success_at` records that a push service accepted a message, and
nothing downstream of that is observable: not whether the browser decrypted it,
not whether the OS displayed it, not whether anyone saw it. A subscription can
therefore look healthy for weeks while delivering nothing — the `403` case is
the one where that is silent by construction.

**Multi-device is untested.** The subscription table admits several rows and the
sender fans out to all of them, but this has only ever run against one browser.
Two devices would raise questions this has no answer for: whether dismissing on
the laptop should silence the phone, and whether "already nudged today" should
be per-device or global. It is currently global.

**DST is accepted, not solved.** Local midnight does not exist on spring-forward
day and exists twice on fall-back day. `local_day_bounds` takes the earliest
valid instant in both cases, so the range is always well-formed, but a nudge on
those two days a year may land up to an hour off. The consequence is bounded and
the alternative is a lot of machinery.

**A cold-start proposal is a guess about a stranger.** `#257` closed with
`#285`: someone who has only ever subscribed is now provably proposed to
rather than passed over, and the end-to-end test above is what holds that
shut. What the epic cannot close is the *quality* of that first proposal.
`recommend()` picks for a subject with no history at all — no completions, no
scores, no abandonment — so the three axes it weighs are, for this one person,
reduced to catalogue order and a daily shuffle. It is a real session at real
floors, and it is not yet an informed one; the first week of
`intervention_log` rows against what those subjects actually open is what
should revise it. The one silence left in the mechanism is narrower and
louder: a catalogue that can compose nothing at all, which
`nudge_waker_nothing_to_say_total` counts and which is a bug rather than a
quiet day.

**The calibration is a guess.** Every weight, half-life, ceiling, and the
threshold itself were chosen by argument rather than by evidence. They are
plausible and they are not tuned; the first week of real `intervention_log` rows
is what should revise them.

**A class removed in a future release strands rows.** `from_discriminant` is a
total parse and unknown discriminants are quarantined, so nothing panics or is
silently reinterpreted — but the stored level is dropped on the floor. A release
that retires a class needs a migration, and there is no mechanism that would
notice if it forgot.

**One subject.** `SubjectId` is a singleton until auth lands. Every table the
study policy reads carries a `subject_id` column as of #259, and every
repository that reads or writes one — including `SessionRepository`, as of
#260 — filters and writes it. What's still singleton is the *value*: every
session route extracts its `SubjectId` via `SubjectId::from_request_parts`
(#261), rather than a handler passing a raw constant, but that extractor still
returns `SubjectId::singleton()` until real auth lands, so nothing has been
exercised with two genuinely different subjects regardless.

**The tag constant lives in three places.** `NUDGE_TAG` (`some-ui.study-nudge`)
is hand-maintained in `file_host::nudge::payload`, in `public/sw.js`, and in
`src/lib/study-nudge/service-worker.ts` — the last two cannot import from each
other because one is a plain `public/` asset. A mismatch shows up as
notifications stacking instead of replacing. Fixing it properly is a `some-ui`
build change.

**A worker update lands one visit late.** A service worker update takes effect
on the visit *after* the one that fetched it. So any change to the payload
contract has to ship in `sw.js` first, and the server may only start relying on
it a deployment later.

---

## Verifying it end to end

Unit tests do not catch the interesting failures here. Several of them look like
success. This is the list worth actually walking, once, on the real machine:

- [ ] Subscribe from the real study browser over HTTPS; confirm the
      `push_subscriptions` row has both keys.
- [ ] `POST /api/v1/push/test` with the browser **closed**; confirm an OS
      notification arrives.
- [ ] Click it; confirm the deep link lands in the session, not the dashboard.
- [ ] `POST /api/v1/signals` with a `session-abandoned` and confirm the returned
      `eligible_at` moves closer; then with a `session-completed` and confirm it
      moves further out. This is the whole engine, observable in two requests.
- [ ] Let engagement actually decay with no signals; confirm the intervention
      arrives near its solved instant and that it is a `LessonReady` rather than
      a coaching message.
- [ ] Verify each silence separately — after completing a session, inside quiet
      hours, after a dismissal, and while the dashboard is connected. Each
      should appear in the logs with its own reason.
- [ ] Revoke the subscription in browser settings; confirm the next send prunes
      the row on `410`.
- [ ] Restart `file_host` immediately after an intervention; confirm no second
      one, and that `engagement_gate.eligible_at` sits in the future.
- [ ] `Ctrl-C`; confirm shutdown does not hang on the tick.
- [ ] Run seven consecutive days and count the notifications against
      `intervention_log`: the refractory floor should be visible in the gaps.

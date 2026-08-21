# The maintainer assay

> Agents may supply implementations and arguments. The maintainer must supply
> independent judgment.

This document evaluates a proposed weekly-challenge workflow against the
epistemic standard it is meant to satisfy, and specifies the amended design.

The proposal's shape is right — weekly cadence, git-versioned prompt, PR as the
interaction surface. What follows is a diagnosis of where it falls short and an
amendment for each, revised once already after a first pass underweighted the
central mechanism.

---

## What is being certified

Not authorship. Not implementation capacity. A senior engineer maintains systems
containing large amounts of code they did not write; agentic coding raises that
fraction without changing the competency criterion.

The claim under test is **counterfactual control**, and it has two independent
axes, not one:

> Can the maintainer detect a bad *change to the code*? Can the maintainer
> detect when a previously sound *implementation is invalidated by a changed
> world*?

Formally: for code `C` and deployment assumptions `A`, an implementation
establishes some invariant `I(C, A)`. The first axis perturbs the code —
`(C, A) → (C', A)` — and asks whether the maintainer would accept `C'`. The
second perturbs the assumptions — `(C, A) → (C, A')` — and asks whether the
maintainer can name the first invariant that stops holding, and why.

The second axis is the one a first pass at this design underweighted, and it
turns out to be the more important one for this specific repository. The reason
is structural, not incidental:

> **This system is deliberately single-user, single-machine, and predominantly
> localhost.** Many of its correctness claims hold only because of that, and the
> repository does not need to be wrong for those claims to be narrower than they
> look.

That gives the assay something the first design didn't have: probe material that
requires inventing nothing. No mutant has to be synthesized, no plausible-looking
bug has to be theorized and then made to actually compile. The material already
exists, in the gap between what the code establishes and what it would need to
establish under a different topology. The exercise is to find that gap without
being told where it is.

---

## The central distinction: hazard, assumption, implementation

Three things that a looser design conflates:

| | What it is | Where it comes from |
|---|---|---|
| **Hazard `G`** | A production failure mode: concurrent writers, duplicate delivery, process death between effect and record, clock skew, partial network failure, version skew, resource exhaustion | An externally grounded, publicly available corpus — not house style |
| **Assumption `A`** | This deployment's actual operating envelope: one process, one SQLite file, one machine, one clock, synchronous local calls | Read off the actual deployment, not invented |
| **Implementation `I`** | What the code does, and is permitted to exploit `A` | The repository |

The hazards in `G` are close to objective. Concurrent writers exist or they
don't; a process can die between an external effect and recording it or it
can't; a network can partition or the code never crosses one. There is far less
room to argue that duplicate delivery isn't real than there is to argue about
which mitigation belongs in a given system. So:

> **The failure modes are objective. Their admissibility and mitigation are
> architectural decisions.**

That line matters because it is what keeps the assay from becoming "did you
recite best practices." The question is never *does `I` satisfy every `g` in
`G`* — reflexively adding a distributed lock to a localhost single-user app
would itself be a wrong answer, over-engineering graded as if it were rigor. The
question is classification:

```
status(g, I, A) ∈ { impossible-under-A, handled-by-I, unhandled-but-acceptable-under-A, latent-defect }
```

For each hazard, the maintainer places it in exactly one bucket and defends the
placement with an execution argument — a concrete sequence of events, not an
assertion. `unhandled-but-acceptable-under-A` and `latent-defect` look identical
from the outside; the only way to tell them apart is whether the maintainer can
say which assumption is being spent and what would have to change in `A` before
it stops being free.

### The other direction: is the coverage bought and paid for?

That classification runs hazard-first: for each `g` in `G`, is it handled. It
has to be run in the other direction too, mechanism-first, or the assay only
ever catches one of the two ways a maintainer can lose the gate.

For each nontrivial piece of machinery `M` in `I` — a message broker, a circuit
breaker, a retry policy, a connection pool sized above what the actual caller
concurrency needs, a distributed cache — name the hazard `M` exists to defend
against, then check that hazard's admissibility under the *actual* `A`, the same
way the hazard-first direction does:

```
provisioning(M, g, A) ∈ { justified, explicit-hedge, unjustified-tax }
```

`justified` means `g` is real under `A` today. `explicit-hedge` means it isn't
yet, but the maintainer can name the near-term change to `A` that would make it
real and the cost of adding `M` later — a documented bet, not an oversight.
`unjustified-tax` means `M` is being paid for continuously — an operational
surface to run and monitor, a new failure mode of its own (the broker being
down is a hazard `A` didn't have before `M` existed), cognitive load on every
reader — against a hazard that is currently `impossible-under-A` with no stated
horizon for that changing.

This is the precise inverse of a latent defect, and it is exactly as diagnostic.
A maintainer who can only produce `handled-by-I` and `latent-defect` findings
and never an `unjustified-tax` finding is one whose relationship to the codebase
is asymmetric in a telling way: comfortable auditing what the code does, unable
to audit what it costs. Reflexively adding a distributed lock to a localhost
single-user app is not rigor, it is the same failure as the latent defect, aimed
the other direction — machinery justified by resemblance to what production
systems do, rather than by a hazard this deployment actually has.

**Candidate targets already in this repository**, named without a verdict —
the verdict is the maintainer's to produce, not the assay's to assert:

- `crates/some-transport` ships both a `nats` feature and an `inmem` feature
  behind the same `Transport` trait — the crate itself already treats
  broker-vs-in-process as a live axis, not a settled one. `NatsTransport` is
  wired into the orchestrator, `some-obs`, and `file_host`. A probe: for each
  call site, is the hazard a broker buys — multi-consumer fan-out, delivery
  across a process crash — real for that call site under this deployment's
  actual `A`, or would `inmem` cover it at lower cost? The crate cannot answer
  this; only the caller's actual topology can.
- `apps/servers/file_host/src/http/layers/circuit_breaker.rs` — a circuit
  breaker defends against a downstream dependency degrading under sustained
  load, which is a hazard of a call graph with a failure domain worth isolating
  from. A probe: what is the failure domain on the other side of this
  particular breaker, and does it have the shape that makes a breaker the right
  primitive, versus e.g. a bounded queue or nothing at all.

Naming these is not a claim that either is wrong. It's the observation that
both are exactly the shape of thing this probe family should be pointed at:
present, plausible, and silently resting on a claim about the world that nobody
has been asked to state out loud recently.

This reframes what "overfit to localhost" means. It is not necessarily a defect.
It is a **claim being made implicitly**, and the question is whether anyone can
state it. Three maintainer states, given the same non-idempotent operation:

1. *"I hadn't noticed it wasn't idempotent."* — no model.
2. *"I know it isn't, but the caller can't redeliver, so nothing exploits it."* —
   a model, but a static one.
3. *"I know it isn't; the invariant depends on exactly-one-invocation inside a
   single process; putting this behind a queue would need an idempotency key or
   a transactional dedup boundary, and here's the interleaving that breaks it
   without one."* — a model with a boundary and a repair.

Only the last two represent actual control, and only the third demonstrates it
at the level this system exists to certify: the maintainer can locate the latent
production boundary, not just gesture at its existence.

---

## Two probe families, not one

The first pass treated mutation-derived refutation as close to the whole
mechanism. It's one instance of a more general move — perturb either side of
`(C, A)` — and for this repository the other side is at least as productive,
because it doesn't compete with `cargo test` for difficulty.

**Implementation perturbation** — `(C, A) → (C', A)`. Apply one semantically
plausible, invariant-breaking edit to real code and present it as what it
resembles: an agent-authored patch. *Accept, reject, or amend — and if you
reject, construct the violating execution.*

Its oracle is mechanical and remains genuinely useful:

| Mutant vs. `cargo test` | Meaning | Use |
|---|---|---|
| fails to compile | the type system is the gate | discard |
| tests fail | CI is the gate | weak, a warm-up at most |
| **tests pass** | **the maintainer is the only gate** | **the challenge** |

A mutation that survives the suite is, by construction, a defect the existing
automation cannot catch — which is exactly the condition under which the
maintainer's judgment is the thing being measured. It also produces a genuine
by-product: a correct answer can end in *now write the test that would have
caught it*, which is the one place this system is allowed to touch live source,
as a maintainer-authored PR, never automatically.

**Environment perturbation** — `(C, A) → (C, A')`. Change one axis of the
deployment topology and ask what breaks, without touching the code at all.
*Under what changed operating assumption does this design's claimed property
stop holding, and what is the smallest execution that demonstrates it?*

This family requires no synthesis of a plausible bug — the source of the first
pass's stated risk that "interesting mutations might all get caught, collapsing
the difficulty ladder." It starts from something known to be true rather than
something that has to be invented: a single-machine system has necessarily been
optimized against a narrow operating envelope, so the curriculum is just that
envelope's axes — cardinality, concurrency, persistence topology, partial
failure, clock authority, retries, isolation, version skew.

Neither family subsumes the other. Mutation tests whether a bad *change* would
get through; environment perturbation tests whether the maintainer's model of
*why the current thing is right* actually extends past the world it was written
in. A maintainer who is purely downstream of an agent can sometimes pass the
first — pattern-matching "this looks wrong" is real signal — and will
characteristically fail the second, because there is no surface artifact to
pattern-match against. The question has no visible wrongness to react to; it has
to be derived.

### Grounded, not hypothetical

Two real examples from this repository, found by reading rather than invented:

**`apps/servers/file_host/src/nudge/waker.rs`** — `run_once` polls
`SELECT subject_id FROM engagement_gate WHERE eligible_at <= now` with no claim
or lock on the returned rows. Correct as long as exactly one waker process ever
runs against the database. `A` here is *single waker instance*. A probe:
*this runs correctly today. What is the first invariant that breaks if a second
replica of file_host starts polling the same database, and what is the smallest
change to `A` that would force a repair — not because two wakers would be a
mistake to run, but because you should be able to say exactly what breaks if
they did.*

**`crates/some-services/src/rate_limiter/token_bucket.rs`** — the bucket's state
is `AtomicU32`/`AtomicU64` fields on the struct: correct per-process coordination,
and free of the complexity a shared store would add, as long as `A` is *one
process serving all traffic*. A probe: *the docs describe this as enforcing
"requests per minute." Under what deployment change does that sentence become
false while every line of this file remains unchanged? Classify the hazard —
`impossible-under-A`, or `latent-defect` waiting for `A` to move.*

Neither of these is a bug report. Both are correct code. The point of each probe
is not to find something to fix; it's to check whether the maintainer's model of
*why* the code is correct extends exactly as far as the code's actual
guarantees, no further and no less.

---

## Verdict on the proposed floor, restated

**Defect 1 — rewrite-in-place discards the deliverable.** Unchanged from the
first pass. "No accumulating files" is right about the *prompt*, wrong about the
*record*. The system's output is a revision-linked, per-concept competency
graph; overwrite the file weekly and there is no graph, only a current question.

**Amendment:** separate the ephemeral prompt from an append-only ledger.

| | Lifetime | Why |
|---|---|---|
| `challenge.md` | rewritten weekly | a stale prompt is clutter |
| `ledger/*.json` | append-only | one small record per probe — this **is** the deliverable |
| `COMPETENCY.md` | regenerated | rendered view of the ledger, examiner-facing |

Fifty-two small records a year is not the accumulation the no-files rule was
written against.

**Defect 2 — the challenge generator needs a real oracle, not one oracle.** The
first pass proposed mutation as close to the sole mechanism, protected by a
cryptographic seal/reveal protocol modeled on an exam with one secret answer.
Given the two-family design above, that protocol is now **downgraded to where it
still earns its cost, and dropped where it doesn't.**

For a mutation challenge, there *is* a specific secret worth protecting — the
mutant's location and the intended violation — so seal/reveal still applies
there in a lightweight form: the mutation diff is not present in the tree the
maintainer checks out; the maintainer's accept/reject/amend is a commit; CI
reveals the diff and grades against it afterward.

For an environment-perturbation challenge, there usually is no single secret
answer worth protecting that way — the rubric evaluates the *reasoning
structure* (was the right hazard named, was the right assumption identified as
the one being spent, was an actual execution constructed), not a hidden
canonical string. Sealing a rubric that consists of "did they reason correctly"
buys little. The cheaper and sufficient protocol is:

> probe fixed → reasoning committed as a PR comment or `answer.md` → critique
> generated from that commit.

Ordering is still enforced — the answer commit's parent still can't contain the
critique — it's just that nothing needs to be cryptographically withheld to get
that property, because there was never a payload worth hiding in the first
place.

**Silence is still evidence.** A challenge that reaches its deadline unanswered
closes with a `declined` record, of either family. Cheap, honest, and it keeps
the ledger from being a highlight reel.

---

## The sharper competency claim

The first pass's operational question was:

> Could a plausible but logically defective agent contribution get through me?

That's still true and still worth asking, but it's now visibly only the
implementation-perturbation half. The complete question:

> **Can the maintainer identify both the invariants an implementation
> establishes and the assumptions that make those invariants true — and predict
> where each stops holding as either the code or the world changes?**

Which compresses to the two sentences that should sit at the top of any future
revision of this document — one per direction, neither optional:

> **Prove that the simplifications are intentional by showing where they stop
> being valid. Prove that the complexity is intentional by showing which hazard
> it buys admission against, and that the hazard is real.**

The first without the second only certifies that the maintainer notices what's
missing. A codebase can fail this system in either direction: it can be
under-defended against a hazard that's actually live, or it can be paying,
continuously, for a hazard that was never live at all — and a maintainer who
cannot tell the difference is not exercising judgment, only pattern-matching
toward whichever direction looks more like "production code."

And it produces a distinction worth stating plainly, because it's the one an
examiner is actually probing for: a system can look identically well-maintained
under both explanations —

- *deliberately specialized*: every simplification is a named, defensible trade
  against a known assumption, and the maintainer can say what would invalidate
  it, or
- *accidentally sufficient*: it simply hasn't been asked the question that would
  expose the gap yet, and neither the maintainer nor the code contains a model
  of why it currently works.

Passive review — reading PRs, nodding along, catching the occasional obvious
error — is compatible with either state. It cannot by itself distinguish them.
That is the precise failure mode named in the original user story: every
individual PR can look reasonable while the maintainer's role quietly stops
being load-bearing. The environment-perturbation family exists specifically
because it is the cheapest available instrument that tells the two states apart.

---

## Architecture

Unchanged in substance: deterministic operations are code, prose is model-
authored, and the two are never allowed to blur.

```
crate  →  selection brief (JSON: concept, anchors, hazard set G, Bloom op, risk score, why)
crate  →  chooses probe family: implementation-perturbation (needs a compiling,
          test-passing mutant) or environment-perturbation (needs one assumption
          in A, read off real deployment config/docs, not invented)
agent  →  challenge.md (+ sealed mutation diff, only for that family) + rubric.md,
          confined to the named anchors and the named hazard set
crate  →  validate schema, verify provenance, append record, render graph
```

The agent never selects its own target and never grades without a rubric or
hazard classification written before it saw the answer. That confinement is what
stops the harness from grading itself into a flattering shape.

### Placement: excluded, not a workspace member

Unchanged. Adding a member to `[workspace]` would gate live CI under
`clippy::pedantic`/`nursery`/`cargo`-deny and move the root `Cargo.lock`.
`.github/scripts/detect` is this repository's own precedent for an excluded,
independently-locked crate — follow it verbatim.

```
.github/scripts/assay/     # excluded crate: selection, ledger, validation, render
.assay/
  challenge.md             # rewritten weekly
  selection.json           # brief the challenge was generated from, incl. probe family
  ledger/*.json            # append-only evidence records
  COMPETENCY.md            # rendered graph, examiner-facing
```

Name: not `quiz`, `tutor`, or `challenge` — each mis-frames the artifact as
pedagogy when it is evidence. `assay` fits: a test of composition, run on a
sample, reported as a measurement. `attest` and `gate` remain acceptable
alternatives.

---

## Cadence and selection

Weekly cron is the liveness floor; what it reads is the part worth getting
right. Selection should weight by **the diff since the last challenge** — a
concept whose supporting source changed since it was last probed is stale by
definition, and should out-compete unprobed concepts. One
`git diff --name-only <last_revision>..HEAD` feeding the risk weight buys most
of the freshness property for free.

Risk weighting asks *what would be expensive to misunderstand*, not *what
haven't we asked about recently*. Signals available in this repository, roughly
by value: a stated invariant in a doc or doc-comment (this repo has several —
`docs/study-nudge.md`'s "discovers rather than decides" is exactly the kind of
claim worth probing on both axes); concurrency or `await` boundaries; any point
where the code assumes single ownership of a resource (the two examples above
are instances of a pattern, not a complete list — a sweep for `SqlitePool`,
`Atomic*`, in-process caches, and `OnceCell`/`lazy_static` state would surface
the rest); transaction and persistence boundaries; commit churn; fan-in;
absence of tests; and whether the code arrived through an agent-co-authored
commit — 26 of the last 100 commits carry a Claude co-author trailer, a directly
usable signal for the exact question this system exists to answer.

Selection should alternate probe family across weeks rather than let one
dominate: implementation-perturbation is bounded by how many mutants survive
`cargo test`, which will thin out over time as the suite improves — a good
outcome for the codebase and a reason not to depend on that family alone.
Environment-perturbation is bounded only by the number of assumptions in `A`,
which does not thin out, because new code keeps introducing new ones.

Environment-perturbation probes should themselves alternate direction: a
hazard-first week (*is this hazard handled*) and a mechanism-first week (*is
this machinery earning its cost*) exercise different, equally necessary halves
of the same judgment. A generator that only ever asks the first question will
train — and only ever produce evidence of — a maintainer who adds defenses on
request and never removes one that stopped making sense.

---

## What this deliberately does not do

Detect authorship. Measure test coverage. Gamify anything. Substitute for
review. Produce a global score — competence is heterogeneous, and "Maintainer
level: Expert" is a claim this design refuses to make. Reward reflexive
production-hardening in either direction — correctly classifying a hazard as
`unhandled-but-acceptable-under-A`, or a mechanism as `unjustified-tax`, is each
a *pass*, not a near-miss, and a rubric that can't represent either as fully
correct is a bad rubric. It does not have a house preference for either more
defensive code or less of it; it has a preference for the maintainer being able
to say which one the codebase currently is, and why. And it does not attempt to
prove assistance was never used; it proves something more defensible — that on
the repository's material logical claims there exists revision-linked evidence
of independent judgment, on what the code does, what the world lets it assume,
and what it's paying for that assumption not holding, recorded before the
answer was free to look up.

---

## Acceptance criteria for v1

1. Indexes the repository at a specific revision and records it in every
   artifact.
2. Selects a target by risk weight and staleness, not rotation, and emits the
   brief — including chosen probe family and, for environment probes, the
   specific assumption in `A` being perturbed — as JSON before generation.
3. Supports both probe families: a mutation whose mutant compiles and survives
   the existing test suite, and an assumption-perturbation whose target
   assumption is read off real deployment config or docs, not invented. Within
   assumption-perturbation, supports both directions — hazard-first (is a real
   hazard handled) and mechanism-first (does existing machinery defend against
   a hazard that is actually admissible under `A`) — and does not let one
   direction dominate selection.
4. For mutation probes, withholds the diff from the tree the maintainer checks
   out; for environment probes, no withholding is required — the rubric grades
   reasoning structure, not a hidden string.
5. Records the maintainer's response as a commit or PR comment whose parent
   demonstrably predates the critique, for both families.
6. Evaluates against a rubric or hazard classification written before the
   answer existed, and records the specific reasoning node demonstrated *and*
   the one missing — never a score alone. `unhandled-but-acceptable-under-A`
   and `unjustified-tax` are each scoreable as fully correct when defended with
   an execution argument or a named, currently-inadmissible hazard.
7. Appends one evidence record per probe; never overwrites the ledger.
8. Renders a per-concept graph with per-concept staleness against the current
   revision.
9. Records `declined` for unanswered challenges of either family.
10. Adds no member to `[workspace]` and does not modify the root `Cargo.lock`.

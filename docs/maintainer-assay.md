# The maintainer assay

> Agents may supply implementations and arguments. The maintainer must supply
> independent judgment.

This document evaluates a proposed weekly-challenge workflow against the
epistemic standard it is meant to satisfy, and specifies the amended design.

The proposal's shape is right. Three of its properties, left as stated, would
cause it to fail its own purpose. What follows is the diagnosis, the amendment,
and the reason the amendment is cheap.

---

## What is being certified

Not authorship. Not implementation capacity. A senior engineer maintains systems
containing large amounts of code they did not write; agentic coding raises that
fraction without changing the competency criterion.

The claim under test is **counterfactual control**:

> Remove the implementation agent. Can the maintainer still predict,
> interrogate, reject, repair, and deliberately evolve the system?

And its sharpest operational form — the one that generates good challenges:

> Could a plausible but logically defective agent contribution get through me?

That question is falsifiable, which is why it is the right one. There really is
a fact of the matter about whether a state transition is reachable, whether an
operation is atomic, whether a type establishes the invariant it claims, whether
a bisection converges to the value the caller assumes, or whether a retry admits
duplicate effects. Judgment about *whether an invariant is desirable* is
contextual and hard to score. Judgment about *whether it holds* is not. The
assay lives in the second register.

---

## Verdict on the proposed floor

The proposed flow:

> agent generates challenge → maintainer commits or does nothing → workflow
> opens a PR → weekly cron reads the PR and history → generates the next
> challenge → lands on main; no files accumulate, the challenge is rewritten in
> place.

**Keep:** the weekly cadence, the git-versioned prompt, the PR as the interaction
surface, the read-back of the maintainer's commits as input to the next probe,
and the strict separation from live source. Those are all correct and they are
the load-bearing half of the design.

**Amend:** three properties, below. None of them is a rewrite. Each is a
structural change small enough to state in a sentence, and each is the difference
between an artifact an examiner accepts and one they do not.

---

### Defect 1 — rewrite-in-place discards the deliverable

"We are not accumulating any files; on a cron we rewrite with a new challenge."

The instinct is right about the *prompt* and fatally wrong about the *record*.
The thing being built is an evidence system. Its output is a revision-linked,
per-concept competency graph with staleness. Overwrite the file weekly and there
is no graph — only a current question and a `git log -p` that no examiner will
reconstruct on your behalf.

**Amendment: separate the ephemeral prompt from the append-only ledger.**

| | Lifetime | Why |
|---|---|---|
| `challenge.md` | rewritten weekly | a stale prompt is clutter; nobody re-reads last month's question |
| `ledger/*.json` | append-only | one small record per probe; this **is** the deliverable |
| `COMPETENCY.md` | regenerated | a rendered view of the ledger, for the examiner |

The ledger does not grow objectionably: fifty-two records a year, each a few
hundred bytes. The no-accumulation rule survives where it was actually about
hygiene, and is dropped where it was about the product.

---

### Defect 2 — nothing makes prediction precede observation

This is the serious one.

The standard the harness is built to meet says: *prediction must precede
observation*, otherwise the exercise tests post-hoc explanation rather than
possession of a predictive model. The proposed flow has no commitment point. The
challenge lands on main, the maintainer works in their own commits, and a week
later the agent reads the result. Nothing in that sequence distinguishes "I
predicted the behavior" from "I read the source, ran it, asked an agent, and then
wrote a correct-sounding paragraph."

An examiner will ask exactly this, and the honest answer under the floor design
is *nothing prevents it*, which collapses the artifact.

**Amendment: let the git DAG carry the ordering proof.**

The challenge ships with its ground truth withheld — not present in the tree at
all. The protocol is two-phase, and each phase is a commit:

1. **Seal.** The weekly workflow opens a PR containing `challenge.md` and
   `rubric.sha256`. The rubric itself lives only in the workflow's encrypted
   output or on an orphan ref; the tree the maintainer checks out does not
   contain it.
2. **Commit.** The maintainer writes `answer.md` and commits it to the PR branch.
   That commit is the commitment. Its parent provably does not contain the
   rubric.
3. **Reveal.** The commit triggers CI, which publishes `rubric.md`, verifies it
   against the sealed hash, and posts the evaluation as a review.
4. **Seal the record.** Merging the PR appends the evidence record to the ledger
   on main.

The DAG then testifies to the ordering: the answer's tree hash predates the
rubric's introduction, and both are signed by CI's timestamps.

This is not tamper-proofing — you hold the keys to your own repository, and any
scheme that pretends otherwise is theatre. It is something more useful: the
ordering is *recorded by default*, so honest use produces a credible artifact
without effort, and dishonest use requires deliberate history manipulation that
leaves its own trace. That asymmetry is the entire point, and it is what an
examiner is actually looking for.

A corollary worth stating: **silence is evidence too.** A challenge PR that
reaches its deadline with no answer commit closes with an evidence record of
`declined`. That is honest, it is cheap, and it prevents the ledger from becoming
a highlight reel.

---

### Defect 3 — "generate a challenge" will decay into repository trivia

Underspecified, an LLM asked weekly to "generate an interesting challenge from
this repo" converges on comprehension questions: *what does this module do, why
is this field an `Option`, trace this call path.* Those are answerable by reading,
which means they test familiarity, not gatekeeping. The anti-goals list already
names this failure; the floor design contains nothing that prevents it.

**Amendment: make refutation the primary challenge class, and derive it from
mutation.**

Rather than generating questions from correct code, generate a **nearby wrong
world**. Take a real subsystem and apply one semantically plausible, invariant-
breaking edit — move an operation across an `await`, weaken an ordering
constraint, alter an error arm, widen a transaction boundary, change a bound,
replace a structure with a superficially adequate one. Present it as what it
resembles: a plausible agent-authored patch. Then:

> Accept, reject, or amend. If you reject, name the invariant violated and
> construct the execution that violates it. Then state the strongest correctness
> argument *against your own conclusion*.

This is the exercise that matches the actual job. It also has three properties
the trivia mode lacks: the ground truth is known by construction, the difficulty
is tunable, and — most importantly — it is mechanically gradable in part.

#### The difficulty oracle

This repo already contains the instrument that grades the challenge before the
maintainer sees it. Build the mutant and run the suite:

| Test suite result | What it means | Use |
|---|---|---|
| Fails to compile | the type system is the gate | discard, not a challenge |
| Tests fail | CI is the gate | weak challenge; a warm-up at most |
| **Tests pass** | **you are the only gate** | **the challenge** |

A mutation that survives `cargo test` is precisely a defect that the repository's
existing automation cannot catch — which makes the maintainer's judgment the sole
remaining barrier, which is the exact claim under test. No model judgment is
required to make this determination, and the surviving-mutant set is the honest
difficulty ladder.

It also produces a valuable by-product. Every surviving mutant is a gap in the
test suite. A challenge answered correctly can end with: *now write the test that
would have caught it.* That is the `Create` rung of the Bloom axis, it is
genuinely useful work, and it is the one place where this pedagogical system is
permitted to touch live source — as a maintainer-authored test, through the
normal PR process, never automatically.

`crates/intervention/src/charge.rs` is the ideal first target: closed-form decay,
a bisected crossing instant, and constants (`MAX_HORIZON_DAYS = 30`,
`BISECTION_STEPS = 40`) whose values are load-bearing for a claim stated in prose
in `docs/study-nudge.md` — that the waker *discovers* rather than *decides*.
Perturb the horizon, the step count, the monotonicity assumption, or the
starting-full choice, and you get four distinct challenges with four distinct
correct refutations, none of which are answerable by reading.

---

## Architecture

The deterministic properties must be code; only the prose should be model-authored.

Provenance, risk weighting, staleness detection, schema validation, and graph
rendering are all mechanical. If an agent performs them, they are assertions. If
a crate performs them, they are checkable — and the crate is itself an artifact
the maintainer designed, which is not nothing.

**The crate owns selection and the ledger. The agent owns the prose.**

```
crate  →  selection brief (JSON: concept, anchors, Bloom op, risk score, why)
agent  →  challenge.md + rubric.md, confined to the named anchors
crate  →  validate schema, verify provenance, append record, render graph
```

The agent never chooses the target and never decides whether an answer counted
without a rubric it wrote before seeing the answer. That confinement is what
stops the harness from grading itself into a flattering shape.

### Placement: excluded, not a workspace member

The stated requirement is that this influence no landed code. Adding a member to
`[workspace]` violates it directly: the workspace denies `clippy::pedantic`,
`nursery`, and `cargo` across all members, so a pedagogy crate would gate live
CI, and its dependency additions would move `Cargo.lock` for the real binaries.

The repository already solved this exact problem. `.github/scripts/detect` is a
standalone Rust crate under `exclude = [".github/scripts"]`, with its own
`Cargo.lock`, built and cached by its own workflow. Follow that precedent
verbatim.

```
.github/scripts/assay/     # excluded crate: selection, ledger, validation, render
.assay/
  challenge.md             # rewritten weekly
  selection.json           # the brief the challenge was generated from
  ledger/*.json            # append-only evidence records
  COMPETENCY.md            # rendered graph, examiner-facing
```

On the name: it should not contain `quiz`, `tutor`, or `challenge` — each
mis-frames the artifact as pedagogy when it is evidence. `assay` fits the
repository's register and means the right thing: a test of composition, run on a
sample, reported as a measurement. `attest` and `gate` are acceptable
alternatives.

---

## Cadence

Keep the weekly cron. Change what it reads.

The floor selects from "the repository." Selection should instead be weighted by
**the diff since the last challenge**, because competence evidence decays exactly
where the repository moved. A concept whose supporting source changed since it
was last demonstrated is stale by definition, and stale concepts should
out-compete unprobed ones. This is one `git diff --name-only <last_revision>..HEAD`
fed into the risk weighting — no new machinery, and it buys most of the freshness
property outright.

Weekly cron is the liveness floor. The stronger trigger — probe on merge of any
PR that touches a subsystem carrying a material invariant — is a natural second
phase, but is not needed to make v1 correct.

Risk weighting should ask *what would be expensive to misunderstand*, not *what
have we not asked about recently*. Available signals in this repository, roughly
in order of value: presence of a stated invariant in a doc or doc-comment;
concurrency or `await` boundaries; transaction and persistence boundaries; commit
churn; fan-in from the dependency graph; absence of tests; and whether the code
arrived through an agent-co-authored commit — 26 of the last 100 commits carry a
Claude co-author trailer, which is a directly usable signal for the precise
question this system exists to answer.

---

## What this deliberately does not do

It does not detect authorship, measure coverage, gamify anything, or substitute
for review. It does not produce a global score — competence is heterogeneous and
"Maintainer level: Expert" is a claim the design refuses to make. And it does not
attempt to prove that assistance was never used; it proves something better and
more defensible, which is that on the repository's material logical claims there
exists revision-linked evidence of independent judgment recorded before the
answer was available.

---

## Acceptance criteria for v1

1. Indexes the repository at a specific revision and records it in every artifact.
2. Selects a target by risk weight and staleness, not rotation, and emits the
   brief as JSON before generation.
3. Generates a mutation-derived refutation challenge whose mutant compiles and
   survives the existing test suite.
4. Withholds the rubric from the tree the maintainer checks out.
5. Records the answer as a commit whose parent demonstrably lacks the rubric.
6. Evaluates against a rubric written before the answer existed, and records the
   specific reasoning node demonstrated *and the one missing* — never a score
   alone.
7. Appends one evidence record per probe; never overwrites the ledger.
8. Renders a per-concept graph with per-concept staleness against the current
   revision.
9. Records `declined` for unanswered challenges.
10. Adds no member to `[workspace]` and does not modify the root `Cargo.lock`.

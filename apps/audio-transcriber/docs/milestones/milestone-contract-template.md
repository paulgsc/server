
# Milestone Contract Template

**Purpose:** the execution-ledger counterpart to the ADR set. ADRs record *why* a
decision was made. This file records *what an autonomous loop must do, how it knows it is
done, and when it must stop and escalate.* One is a rationale layer, the other a control
layer. They cross-link; neither subsumes the other.

An agentic loop consumes a milestone by reading exactly five things and ignoring prose:
its **entry preconditions** (may I start?), its **exit gate** (am I done?), its
**invariants under test** (is the gate testing the right thing?), its **abort trigger**
(must I stop and hand back to a human?), and its **edges** (what does finishing unblock?).
Everything an ADR is good at — narrative, alternatives, consequences — stays in the ADR
and is referenced, not duplicated.

---

## Schema

Every milestone is one record with these fields. Fields are mandatory unless marked
*(optional)*. A milestone missing a non-optional field is not executable and the loop
must refuse to start it.

```
ID            <kind>-<n>-<slug>      # kind ∈ {build, decision, measure, integrate}
TITLE         one line
STATUS        proposed | ready | active | gated | blocked | done | aborted
PREMISE       the user-story invariant this milestone serves, and the ADR it links to.
              If you cannot state the felt outcome that degrades when this is skipped,
              the milestone does not belong in the roadmap.
DEPENDS_ON    [milestone IDs]        # inbound DAG edges; all must be `done`
BLOCKS        [milestone IDs]        # outbound edges (informational, derivable)
ENTRY         machine-checkable preconditions. Each is a command or a referenced
              `done` milestone. Loop verifies ALL before STATUS may become `active`.
DELIVERABLE   the durable thing produced. For build: an artifact + passing gate.
              For decision: a resolved fork written to a named path.
              For measure: a recorded number bound to a threshold.
GATE          the flavored, deterministic done-check. See "Gate flavors" below.
              A gate is a command (or set) that returns pass/fail with no human judgment
              in the loop's happy path. Non-deterministic gates are defects.
INVARIANTS    the specific properties the gate verifies, in plain language, so a human
              can audit that the gate covers them. Gate ⊆ invariants is a smell;
              invariants ⊄ gate means the gate is incomplete.
ABORT         the condition under which the loop must STOP, mark `aborted`, and escalate.
              This is the autonomy guardrail. A milestone with no abort trigger is a
              milestone the loop can grind on forever. Every fork must have one.
ARTIFACTS     files created/touched. The manifest a human diffs and the loop tracks.
OPEN_Q        *(optional)* carried questions. Explicit, never silently dropped. If an
              open question would change the gate, it is not optional — it is an ABORT.
```

### Gate flavors

A gate names its flavor so the loop knows how to run it and a human knows how to trust it.

| flavor          | mechanism                                            | returns                       | use for                                  |
|-----------------|------------------------------------------------------|-------------------------------|------------------------------------------|
| `test`          | runs a test target, exit code                        | 0 / non-zero                  | build milestones, behavioral correctness |
| `static`        | grep / forbidden-import / lint / AST check           | clean / violations            | invariant enforcement (e.g. no `tokio::` under `hotpath/`) |
| `measure`       | runs a probe, emits a metric, compares to threshold  | metric vs bound               | latency-budget stages, RTF, jitter       |
| `decision-record` | asserts a doc exists at a path with required fields | present+complete / missing    | decision milestones (fork resolution)    |
| `human-signoff` | explicit person approval                             | approved / not                | irreversible or high-blast-radius steps  |

Concurrency-ordering proofs are a sub-case of `test` but must use **exhaustive interleaving
(loom)**, never a stress loop. A stress loop that passes is not evidence; it is the absence
of counter-evidence. State the tool in the gate, not "stress."

### Status transitions

```
proposed → ready        when PREMISE + DEPENDS_ON are filled and edges resolve
ready    → active       when ENTRY all pass
active   → gated        when DELIVERABLE exists, awaiting GATE
gated    → done         when GATE passes
gated    → active       when GATE fails (retry within budget)
any      → blocked      when a DEPENDS_ON regresses
any      → aborted      when ABORT fires → escalate, do not retry
```

`aborted` is terminal for the loop. Only a human moves it back to `ready`.

---

## Instantiation — `decision-0-ingestion-boundary`

This is the first milestone in the roadmap and it is a **decision**, not a build. It exists
because the live codebase and the ADR set describe two different machines, and nearly half
the roadmap's load-bearingness depends on which one is real.

```
ID          decision-0-ingestion-boundary
TITLE       Resolve audio ingestion topology; re-home every ADR invariant to a named binary
STATUS      ready
```

**PREMISE**
Two user-story invariants both hinge on one unanswered question — *where does the microphone
sit relative to the GPU?*

- The mic→text latency budget's stage list (how many hops, which one dominates) cannot be
  written until hop count is known.
- The entire `hotpath/` real-time discipline (ADR-001..004: zero-alloc buffer, executor-free
  SPSC, stack-only kNN) is justified *only* by the premise in ADR-001 that "Tokio can park a
  real-time audio callback on any OS thread." If there is no in-process audio callback on the
  GPU box, that premise does not apply on the box, and M1–M4 are over-engineering *there*.

Until topology is pinned, the loop cannot tell whether building M1–M5 serves the story or
polishes a part no felt outcome depends on. Links: ADR-001 (the callback premise), ADR-005
(GPU pre-alloc, topology-independent), ADR-006 (integration, currently assumes one binary).

**DEPENDS_ON** `[]` — this is the root. Everything downstream waits on it.

**BLOCKS** `measure-0-latency-budget`, and the re-home/keep/delete disposition of
`build-1`..`build-5`.

**ENTRY**
The contradiction must be acknowledged in-repo before resolution starts (so the loop is
resolving a stated fork, not discovering one mid-flight):

1. `static`: confirm the three facts that constitute the fork are present —
   `grep -q '^cpal' apps/audio-transcriber/Cargo.toml` (declared) **and**
   `! grep -rq 'cpal::' apps/audio-transcriber/src/` (unused) **and**
   live ingestion is NATS (`grep -rq 'AudioChunk.*subject\|audio.chunk' src/`).
   All three true ⇒ the codebase currently embodies a network hop while the ADRs describe
   an in-process callback. Fork confirmed.
2. ADR-001..005 are `Accepted` (they are).

**DELIVERABLE**
A new decision record `docs/adr/ADR-000-topology.md` (numbered 000 because it is logically
prior to ADR-001's isolation decision). It resolves the fork to exactly one topology and
produces a per-binary invariant assignment table.

The decision space:

- **A — Co-located capture.** Mic on the box; `cpal` runs an in-process RT callback; GPU
  same process. M1–M4 RT discipline lives in this one binary. The NATS `audio.chunk` subject
  becomes process-internal or is deleted. ADR-001's premise holds as written.
- **B — Distributed thin client.** Mic on a remote client that encodes and ships chunks over
  LAN via NATS. The box has **no** RT callback; it receives already-heap, already-network-
  jittered `Vec`s. On the box, M1–M4 alloc/SPSC discipline is noise against network +
  accumulation jitter and should be **deleted from the box** (or never built there). ADR-005
  stays load-bearing on the box; RT discipline, if it exists anywhere, belongs on the client.
- **C — Split binaries (suspected actual intent).** A `capture` client binary owns `cpal` RT
  capture + M1–M4. The `transcriber` box binary owns NATS ingestion + ADR-005 + ADR-006. The
  ADRs currently fuse both binaries' invariant sets into one `src/hotpath/` tree; C forces the
  split and re-homes each ADR to its true binary. This is the only option consistent with
  *both* "own mic" *and* "lan/nixos server box" in the story as written.

**GATE** — two parts, both must pass.

1. `measure` — quantify the hop, so the choice is evidence-driven not vibes-driven.
   Run `tools/probe-nats-jitter.*`: publish N timestamped `audio.chunk` messages from a
   client on the target LAN, record inter-arrival delta distribution on the box. Emit
   p50/p99 jitter in ms. **Decision rule, encoded:** allocator jitter is sub-millisecond;
   if p99 network jitter ≫ that (it will be, on any real LAN), then on the box RT-alloc
   discipline is *provably* dominated noise → topology is B or C, never A-on-the-box.
   Threshold: p99 ≥ 5 ms ⇒ box RT-alloc discipline is non-load-bearing.

2. `decision-record` — assert `docs/adr/ADR-000-topology.md` exists and contains all required
   headers: `## Chosen Topology`, `## Per-Binary Invariant Assignment` (a table mapping each
   of ADR-001..006 to exactly one binary), `## Dominant Latency Source Per Hop`, and
   `## Disposition of src/hotpath/ on the box` (keep / migrate-to-client / delete).
   Static check greps for each header and for one binary-assignment row per ADR.

**INVARIANTS**

- Each ADR-001..006 invariant is assigned to **exactly one** named binary. No orphan
  (unassigned), no shared (an invariant owned by two binaries is unenforceable).
- `cpal` appears in a `Cargo.toml` **iff** that crate's binary captures audio. Topology B
  with `cpal` still in the box's manifest is a gate failure.
- No single audio path simultaneously claims "NATS-ingested heap `Vec`" and "zero-alloc RT
  callback." That is the present contradiction; the gate must reject any resolution that
  preserves it.
- The chosen topology's hop count equals the number of stages the latency budget
  (`measure-0`) will enumerate. The decision record and the budget cannot disagree on hop
  count.

**ABORT**

- The probe shows network jitter is **not** dominant (genuinely co-located capture, or an
  RT-tight transport) **and** topology A-on-the-box remains viable. The fork is then
  truly open on the merits → mark `aborted`, escalate. **The loop must not coin-flip an
  architectural topology.** A 50/50 fork is a human decision by construction.
- The probe cannot run (no client on the LAN to publish from, no box to receive) → the
  measurement input is unavailable → `aborted`, escalate; do not resolve the fork blind.

**ARTIFACTS**

- creates `docs/adr/ADR-000-topology.md`
- creates `tools/probe-nats-jitter.*` (the measurement probe; owned crate, no new deps)
- annotates `apps/audio-transcriber/Cargo.toml` (justify or remove `cpal` per chosen topology)
- (on resolution to B or C) flags `src/hotpath/` for re-home or deletion — executed by a
  downstream build milestone, *recorded* here

**OPEN_Q**

- Does the capture side run VAD at the edge (push `vad.rs` to the client to save LAN
  bandwidth under B/C)? Affects where ADR's VAD invariant homes. Carried.
- Core NATS vs JetStream for the `audio.chunk` subject — at-most-once vs at-least-once
  changes the drop-counting semantics referenced in ADR-002/003. Carried; does not gate
  this milestone but feeds `integrate` milestones.
```

---

## Notes for the next piece

`measure-0-latency-budget` is the natural successor and is **blocked** on this milestone:
its stage list is `(hops from chosen topology) × {capture, accumulate, transfer, infer,
publish}`, and its dominant-stage target is what M1–M5 should actually be optimizing
against. Do not cut the budget until `decision-0` is `done` or `aborted`-then-resolved.

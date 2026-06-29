
# Milestone — `measure-0-latency-budget`

Second instantiation of the milestone-contract schema (see
`milestone-contract-template.md`). Successor to `decision-0-ingestion-boundary`,
which resolved to **topology B (thin client ships chunks over NATS)**.

This is a `measure` milestone. Its product is not code — it is the evidence that tells the
loop which downstream build milestones are load-bearing and which are noise to be deleted.

```
ID          measure-0-latency-budget
TITLE       Decompose mic→text into per-stage target-ms under B; prove the dominant pair
STATUS      ready
```

**PREMISE**
The binding user-story invariant is *felt* mic→text latency. M1–M4 optimize allocator
jitter (sub-millisecond). Under B the box has no real-time path, so this milestone's job is
to measure where the felt seconds actually go and thereby decide which build milestones earn
their place. It supersedes the implicit budget buried as an unchecked RTF box in ADR-006,
and it sets the optimization target ADR-005 (GPU pre-alloc) is the lever for. Felt latency
under B factors as:

```
mic→text ≈ accumulation_window
          + network(client→box)
          + decode
          + RTF × accumulation_window      ← inference
          + network(box→consumer)
```

With defaults (3 s window) and a GPU RTF ~0.12, the first and fourth terms are ~3000 ms and
~360 ms; every other term is single-to-tens of ms. The whole roadmap's optimization energy
should follow that ratio.

**DEPENDS_ON** `[decision-0-ingestion-boundary]` — `done`, topology = B, hop count fixed.

**BLOCKS**
- disposition of `build-1`..`build-4` (the `hotpath/` tree): this budget supplies the
  proof they are non-load-bearing on the box.
- tuning targets for `build-5` (ADR-005 GPU pre-alloc).
- `decision-1-windowing` (the accumulation-window lever), which this budget motivates.

**ENTRY**
1. `decision-0` is `done` with chosen topology = B.
2. `static`: the 10-stage pipeline is enumerable from real code locations under B —
   client-capture · client-encode · NATS-leg-in · box-decode (`decode_samples`) ·
   accumulate (`take_buffer_if_ready`, gated on `buffer_capacity`) · VAD
   (`contains_speech`) · enqueue (`queue_tx.try_send`) · infer (`process_transcription_job`)
   · publish (`publish_segments_sync`) · NATS-leg-out. Every stage maps to a line or is
   tagged `client-owned, not instrumented`.

**DELIVERABLE**
`docs/adr/ADR-007-latency-budget.md` containing a stage table — one row per stage with
`{owner-binary, p50 ms, p99 ms, measured|estimated, is-config-knob, optimization-lever}` —
plus a stated end-to-end target and an explicit **dominant-pair** declaration backed by
measured ms.

**GATE** — two parts, both must pass.

1. `measure` — instrument end-to-end using the OTel histograms **that already exist** (owned
   substrate, no new deps): `chunk_processing_latency`, `resampling_latency`,
   `transcription_queue_latency`, `transcription_processing_latency`, `transcription_latency`,
   and critically `transcription_end_to_end_latency` — which is **declared in
   `observability.rs` and never `.record()`-ed anywhere**. Wiring it is a precondition of
   measuring the one number the user actually feels. Run `tools/trace-e2e.*` to drive a real
   client→box→consumer trace; emit p50/p99 per stage.
   **Decision rule, encoded:** the budget is valid iff `Σ stage-p50 ≈ measured end-to-end p50`
   within 10%. A gap means a hidden stage; gate fails until the table accounts for it.

2. `decision-record` — `docs/adr/ADR-007-latency-budget.md` exists with required headers
   `## Stage Budget`, `## End-to-End Target`, `## Dominant Pair`, `## Disposition Evidence
   for hotpath/`. A static check greps the headers and asserts one stage-row per pipeline
   stage.

**INVARIANTS**
- Every budget stage maps to a code location or is explicitly `client-owned, not
  instrumented`. No phantom stages, no unattributed time.
- `transcription_end_to_end_latency` is actually recorded. The budget is unmeasurable
  otherwise; this is the load-bearing wiring gap, not a nicety.
- The dominant pair `{accumulation_window, inference}` accounts for ≥ 90% of end-to-end p50.
  If it does not, the roadmap's premise is wrong and this milestone must abort rather than
  publish a budget that mis-ranks the levers.
- The allocator-sensitive stages (`accumulate`, `enqueue`) are shown to contribute < 1% of
  end-to-end p99. This row **is** the disposition evidence that `build-1`..`build-4` are
  non-load-bearing on the box.

**ABORT**
- The dominant pair is **not** `{accumulation, inference}` — e.g. NATS-leg p99 is pathological
  on the target LAN, or `decode_samples` is unexpectedly heavy. The roadmap's lever-ranking
  inverts → mark `aborted`, escalate, re-derive which build milestones matter before any are
  touched. The loop must not publish a budget it has just disproven.
- End-to-end cannot be traced (no live client to drive client→box→consumer) → measurement
  input unavailable → `aborted`, escalate. Same input-availability guard as `decision-0`.

**ARTIFACTS**
- creates `docs/adr/ADR-007-latency-budget.md`
- flags wiring of `transcription_end_to_end_latency` in `src/worker/whisper.rs` + `src/main.rs`
  (recorded here; executed by a build milestone — the metric is declared in
  `src/observability.rs` but never populated)
- creates `tools/trace-e2e.*` (drives and reduces a client→box→consumer latency trace)

**OPEN_Q**
- **Sliding / overlapping windows with partial emits.** The only lever that cuts the
  ~3000 ms accumulation term without starving Whisper of context. Absent from every ADR. This
  is the highest-leverage unbuilt idea under B and should become `decision-1-windowing`,
  blocked on this budget. Carried, flagged as top priority.
- Does the thin client need RT capture discipline at all? Phone/browser mic APIs buffer for
  you; if so, M1–M2 migrate nowhere and simply die. Determines whether ADR-001/002 survive
  on the *client*. Carried.
- `buffer_fill_time` histogram is also declared-and-never-recorded — fold into the same
  wiring pass or delete. Carried.
```

---

## Note for the next piece

The budget's own OPEN_Q names the successor: `decision-1-windowing`. Once this milestone
proves accumulation is ~85–90% of the felt latency, the next decision is whether to keep the
fixed 3 s window (simple, high-latency) or move to a sliding window with partial-emit
re-transcription (complex, the real latency lever) — a fork with a quality-vs-latency
trade-off that, like `decision-0`, the loop must not resolve by coin-flip.

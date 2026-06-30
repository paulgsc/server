# some-cache

Shared caching contract for the bins in `paulgsc/server`. One source of truth for
the on-wire Redis envelope and the in-process dedup layer in front of it, so the
axum server and the pipeline daemon never disagree about formats again.

## Layers

| Type | Role |
|---|---|
| `CacheEntry<T>` / `BinaryCacheEntry` | on-wire envelope stored in Redis |
| `CacheStore` | generic get/set/delete with retry + zstd compression |
| `DedupCache` | in-process thundering-herd guard over `CacheStore` |
| `CacheConfig` | construction parameters (no bin-specific deps) |
| `inproc::*` | hand-rolled single-flight cache backing `DedupCache` (see below) |

Each bin still owns: building `CacheConfig` from its own config, its own richer
error enum (`#[from] DedupCacheError` / `CacheError`), and domain-specific Redis
ops that don't belong in a generic cache.

---

## Active work: M2 "mechanical sympathy" — `inproc`

> Milestone: https://github.com/paulgsc/server/milestone/2

`DedupCache`'s in-process single-flight guard is being rebuilt **by hand** from
`std` + `tokio`, replacing the `moka` and `fastrand` dependency trees. This is a
deliberate learning sprint run as a tutored exercise, not a rewrite for its own
sake — the goal is a leaner dependency graph *and* real fluency in the systems
concepts the dependency was hiding.

The `src/inproc/` module is currently a **skeleton**: every method is `todo!()`
with a doc comment naming the concept, and every requirement is encoded as a test
in `tests/`. Implement it milestone by milestone.

- **Where to start:** [`docs/USER_STORY.md`](docs/USER_STORY.md) — the user story,
  the epic, and the six stories (M2.0–M2.5) with their Dreyfus/Bloom framing.
- **What each milestone means:** [`docs/ROADMAP.md`](docs/ROADMAP.md).
- **When a story is done:** [`docs/DEFINITION_OF_DONE.md`](docs/DEFINITION_OF_DONE.md).
- **Why we're doing it:** [`docs/adr/ADR-001.md`](docs/adr/ADR-001.md).

### The invariant contract (`inproc`)

The module carries a scoped `#![deny(...)]` lint block and is held to:

- **Zero heap allocation** on steady-state `get`/`insert` and the probability gate
  (proven under `tests/alloc_guard.rs`'s guarded allocator).
- **Cache-line padding** — each `Shard` is `#[repr(align(64))]` so adjacent shard
  locks never false-share.
- **Hard capacity bound** — CLOCK eviction never grows past capacity under churn.
- **Single-flight + cancellation safety** — exactly one fetch under N concurrent
  callers; cancelling the leader must not hang the followers.
- **No `%` on the hot path** — shard selection is a power-of-two mask.

### Running the loop

```bash
cargo test  -p some-cache                                   # structural invariants (green today)
cargo test  -p some-cache milestone_m2_3 -- --include-ignored   # a milestone as you build it
cargo test  -p some-cache --test alloc_guard -- --ignored --test-threads=1   # zero-alloc proofs
cargo clippy -p some-cache                                  # the lint contract has teeth
```

`tests/milestones.rs` is the definitive source of truth for progress: a milestone
is done when its tests pass, its `#[ignore]`s are gone, and the lint contract holds.

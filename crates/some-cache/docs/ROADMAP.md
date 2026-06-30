# ROADMAP — `some-cache` M2 "mechanical sympathy"

The milestone tests in `tests/milestones.rs` and `tests/alloc_guard.rs` are the
source of truth. This file is the human-readable definition behind each one,
plus the benchmark protocol and exit evidence.

Milestone is complete ⇔ all its tests pass, its `#[ignore]`s are deleted, and the
`src/inproc` lint contract still holds (`cargo clippy -p some-cache` clean for
the module).

---

## Dependency graph (do them in order)

```
M2.0 fastrand→rng ──┐
                    ├─→ M2.5 cutover & delete deps
M2.1 trait ─→ M2.2 shards ─→ M2.3 CLOCK ─→ M2.4 single-flight ─┘
```

M2.0 is independent (warmup, ship it first for momentum). M2.1→M2.4 are a chain:
the trait frames the type, shards hold the maps, maps need an eviction policy,
and single-flight sits on top of the assembled cache. M2.5 needs all of them.

---

## M2.0 — Probability gate (replaces `fastrand`)

**Spec.** `inproc::rng::probability_gate(p) -> bool`: `true` with probability
`p`, clamped to `[0,1]`. `p<=0 ⇒ false`, `p>=1 ⇒ true`. Thread-local PRNG
(xorshift64\* / PCG / SplitMix64), non-zero per-thread seed. Zero allocation.

**Tests.** `milestone_m2_0_boundaries_are_deterministic` (green today),
`milestone_m2_0_is_approximately_uniform` (≈p over 100k trials, ±0.02),
`milestone_m2_0_probability_gate_zero_alloc` (alloc_guard).

**Integration.** `store.rs::should_touch` calls it; `fastrand` leaves `Cargo.toml`.

---

## M2.1 — `InProcStore` trait

**Spec.** The minimal sync surface `DedupCache` needs: `get`, `insert`, `remove`,
`invalidate_all`, `len`, `is_empty`. Object-safe. `InProcCache` implements it.
Single-flight is an inherent `async` method on `InProcCache`, deliberately *not*
on the trait.

**Test.** `milestone_m2_1_trait_surface_is_object_safe` (compile-time).

---

## M2.2 — Sharded, cache-line-padded storage

**Spec.** `InProcCache` holds `Box<[Shard]>`, length a power of two, with
`shard_mask = len - 1`. `shard_for(key)` hashes the key once and selects with
`hash & shard_mask`. Each `Shard` is `#[repr(align(64))]` around a
`Mutex<LruMap>`. Per-shard capacity = `ceil(max_entries / shard_count)`.

**Tests.** `milestone_m2_2_shard_is_cache_line_aligned` (green today),
`milestone_m2_2_keys_distribute_across_shards`.

**Benchmark (the lesson).** Add `benches/false_sharing.rs` (criterion, dev-dep) or
a `#[ignore]`d timing test comparing:
- `#[repr(align(64))]` shards vs. a deliberately unpadded `struct ShardUnpadded`,
- N threads each hammering a *distinct* shard.

Expect the padded layout to win materially on multi-core hardware. Record the
numbers in the PR. If there's no gap, investigate before claiming M2.2 — either
the benchmark isn't contended or the layout isn't doing what you think.

---

## M2.3 — Bounded CLOCK eviction

**Spec.** `LruMap`: flat `Box<[Slot]>` (`{ key_hash, value: Arc<[u8]>,
referenced: bool, occupied: bool }`) + `HashMap<u64, usize>` index + a clock
`hand`. `get` sets `referenced`. `insert` at capacity sweeps the hand, clearing
set bits and evicting the first clear slot. `len() <= capacity` always. Warm
`get`/`insert` allocate zero times.

**Tests.** `milestone_m2_3_get_insert_roundtrip`,
`milestone_m2_3_never_exceeds_capacity`,
`milestone_m2_3_referenced_entry_survives_eviction`,
`milestone_m2_3_warm_get_insert_zero_alloc` (alloc_guard).

**Second-chance sequence (the referenced-survival test):** insert 1, insert 2
(cap=2); `get(1)` to set its bit; insert 3 ⇒ hand finds slot 1 referenced
(clear it, skip), evicts slot 2. `get(1)` is still `Some`, `get(2)` is `None`.

---

## M2.4 — Single-flight + cancellation safety

**Spec.** `InProcCache::try_get_with(key, init)`: fast-path `get`; on miss, the
first caller registers a slot and runs `init`; concurrent callers for that key
await the same result. `tokio::sync::Notify` is the suggested primitive (enable
tokio `sync` feature). On success, cache the bytes and wake waiters.

**Cancellation invariant (Expert probe).** If the leader's future is dropped
mid-fetch, an RAII guard must remove the slot and wake waiters so a follower is
promoted and retries — no permanent hang. Document this in a `# Cancellation`
section on `try_get_with`.

**Tests (port into `milestone_m2_4`, needs tokio `rt`/`macros` dev-deps).**
- one fetch under N concurrent callers (`init` invocation counter == 1),
- cancelling the leader (`tokio::time::timeout` / drop) does not hang followers,
- `outstanding()` returns to 0 after completion and after cancellation.

---

## M2.5 — Cutover & delete dependencies

**Spec.** `DedupCache.in_flight: moka::future::Cache<...>` becomes
`InProcCache`. Map `try_get_with`'s `Arc<[u8]>` / error contract onto
`DedupCacheError`. Remove `#![allow(dead_code)]` from `inproc/mod.rs`. Delete
`moka` and `fastrand` from `Cargo.toml`.

**Tests.** `milestone_m2_5_dependencies_removed` + the entire M2.0–M2.4 suite +
existing dedup behavior unchanged.

**Exit evidence (paste into the closing PR):**
- `cargo tree -p some-cache` before/after (the moka subtree is gone),
- the M2.2 false-sharing benchmark numbers,
- the M2.3 zero-alloc proof output,
- one paragraph: what moka was buying, what hand-building cost, what you learned.

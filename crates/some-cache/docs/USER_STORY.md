# User Story & Epic — Mechanical Sympathy in `some-cache`

> Milestone: [**M2 "mechanical sympathy"**](https://github.com/paulgsc/server/milestone/2)
> Pedagogy: the [`rust-jit-tutor`](../../../.claude) skill — conjecture & refutation,
> tracked on Dreyfus (Novice → Expert) and Bloom (Remember → Create) axes.
> This is a **learning sprint**: the deliverable is mastery *demonstrated by* a
> leaner, hand-built cache — not generated code.

---

## The user story

> **As** the maintainer of `paulgsc/server`,
> **I want** the in-process dedup cache rebuilt by hand from `std` + `tokio`
> instead of pulling the `moka` and `fastrand` dependency trees,
> **so that** the crate carries less supply-chain / compile-time weight **and**
> so that I come out the other side genuinely fluent in the systems concepts
> the dependency was hiding from me — sharding, false sharing, cache-friendly
> eviction, and cancellation-safe async coordination.

**Why this framing.** I can already make the tokens appear. What I want from
this milestone is the *other* thing: to be nudged down the right path in
piecemeal, verifiable steps, where each step forces my mental model to make a
prediction and then checks it against `cargo test`. The issues below are that
path. The skeleton in `src/inproc/` (all `todo!()`) is the guard rail so the
project doesn't get abandoned half-built — every stub has a doc comment naming
the concept and a test that defines "done."

**Definition of "expert" here (from the tutor skill).** Not "it compiles."
Expert = *knows when rules can be safely violated and can name what the compiler
cannot prove.* The M2.4 cancellation-safety task is the deliberate Expert probe:
the type system will not catch a leaked single-flight slot — you have to.

---

## Epic

**EPIC — Replace `moka` + `fastrand` with a hand-rolled, cache-line-aware,
single-flight in-process cache.**

Outcome: `DedupCache` runs on `some_cache::inproc::InProcCache`; `moka` and
`fastrand` are gone from `Cargo.toml`; `tests/milestones.rs` and the zero-alloc
tests in `tests/alloc_guard.rs` are green with the `inproc` lint contract
intact; `cargo tree -p some-cache` no longer shows the moka subtree.

The Epic decomposes into six stories, sequenced by *concept dependency* and by
Dreyfus level — each is a self-contained GitHub sub-issue. Do them in order;
each opens the door to the next.

---

### Story M2.0 — Drop `fastrand` (warmup)

**Dreyfus:** Novice → Advanced Beginner **· Bloom:** Apply
**Files:** `src/inproc/rng.rs`, `src/store.rs`

Replace `fastrand::f64() < p` in `should_touch` with a thread-local xorshift64\*
(or PCG / SplitMix64) probability gate. Delete `fastrand`.

- **Conjecture to make before coding:** why is a `thread_local!` `Cell<u64>`
  sound for PRNG state through a shared `&`, with no `Mutex`? (Name the
  invariant: `Cell` is `!Sync`; state never crosses a thread.)
- **Refutation:** `milestone_m2_0_*` — boundary determinism, statistical
  uniformity, and **zero allocation** under `alloc_guard`.
- **Done when:** tests green, `#[ignore]`s gone, `fastrand` removed.

---

### Story M2.1 — Define the `InProcStore` trait

**Dreyfus:** Competent **· Bloom:** Create
**Files:** `src/inproc/mod.rs`, `src/inproc/cache.rs`

Design the trait that captures *exactly* the slice of moka `DedupCache` needs —
no more. Implement it (stubbed) for `InProcCache`. This is the tutor skill's
"design a trait that enforces an invariant" probe, made real.

- **Conjecture:** why does single-flight (`try_get_with`) stay *off* the trait as
  an inherent `async` method, instead of an `async fn` in the trait? (Object
  safety; not needing a `Send` story you can't yet justify — an Evaluate call.)
- **Refutation:** `milestone_m2_1_trait_surface_is_object_safe` (compile-time).
- **Done when:** the trait exists, `InProcCache: InProcStore`, and you can defend
  every method as one moka actually required.

---

### Story M2.2 — Sharded, cache-line-padded storage (false sharing)

**Dreyfus:** Proficient **· Bloom:** Analyze → Evaluate
**Files:** `src/inproc/shard.rs`, `src/inproc/cache.rs`

Build the shard array. Each `Shard` is `#[repr(align(64))]` so two cores hitting
two different shards never contend on one cache line. Select shards with a
power-of-two mask, not `%`.

- **Conjecture:** before benchmarking, predict the throughput delta between
  padded and unpadded shards under multi-core contention, and explain the MESI
  cache-coherence mechanism that causes it.
- **Refutation:** `milestone_m2_2_shard_is_cache_line_aligned` (green today) +
  the padded-vs-unpadded benchmark in `docs/ROADMAP.md`. If the benchmark
  doesn't show the gap, you haven't demonstrated the lesson.
- **Done when:** keys route deterministically, distribute across shards, layout
  invariant holds, benchmark confirms the false-sharing effect.

---

### Story M2.3 — Bounded CLOCK eviction

**Dreyfus:** Proficient → Expert **· Bloom:** Create
**Files:** `src/inproc/lru.rs`

Implement per-shard bounded eviction as CLOCK (second-chance) over a flat
`Box<[Slot]>` + index map — *not* a pointer-chasing doubly-linked-list LRU.

- **Conjecture:** explain why the textbook LRU is cache-hostile (per-node heap
  allocation, 3–4 pointer chases per `get`, each a likely cache miss) and why
  CLOCK approximates it with sequential memory access.
- **Refutation:** `milestone_m2_3_*` — roundtrip, never-exceeds-capacity under
  churn, second-chance survival, and **zero-alloc** warm `get`/`insert`.
- **Done when:** all M2.3 tests green; warm ops allocate zero times.

---

### Story M2.4 — Single-flight + cancellation safety (the hard one)

**Dreyfus:** Expert **· Bloom:** Analyze
**Files:** `src/inproc/single_flight.rs`, `src/inproc/cache.rs`

Implement `try_get_with`: first caller per key is the leader (runs the fetch);
concurrent callers are followers (await the shared result). Then the Expert
probe — **cancellation safety**.

- **Conjecture (answer in prose in the PR before coding):**
  1. At which `.await` can the leader be cancelled?
  2. What state is left behind, and which callers are now stuck forever?
  3. Why does an RAII `Drop` guard fix it where a trailing cleanup line cannot?
- **Refutation:** N concurrent callers ⇒ exactly **one** fetch; cancelling the
  leader does **not** hang followers (a promoted follower retries).
- **Discipline gate:** the method must carry a `# Cancellation` doc section
  stating the safety invariant. Skipping it means M2.4 is *not* done — an
  undocumented invariant is itself the diagnostic.
- **Note:** enables tokio `sync` feature + `rt`/`macros` dev-deps for the tests.

---

### Story M2.5 — Cutover & delete the dependencies

**Dreyfus:** Proficient **· Bloom:** Evaluate
**Files:** `src/dedup.rs`, `src/inproc/mod.rs`, `Cargo.toml`

Wire `DedupCache` to `InProcCache`, remove the `#![allow(dead_code)]`, delete
`moka` and `fastrand`, and compare before/after `cargo tree` + benchmarks.

- **Refutation:** `milestone_m2_5_dependencies_removed` (manifest has no moka /
  fastrand) + full milestone suite green + dedup behavior unchanged.
- **Done when:** the Epic outcome holds and you can articulate, with numbers,
  what the dependency was buying and what hand-building it cost and taught.

---

## How to run the loop

```bash
# Structural invariants (green today):
cargo test -p some-cache

# A milestone's full suite as you implement it:
cargo test -p some-cache milestone_m2_3 -- --include-ignored

# Zero-alloc proofs (process-global guard — single-threaded):
cargo test -p some-cache --test alloc_guard -- --ignored --test-threads=1

# The lint contract has teeth:
cargo clippy -p some-cache
```

Bring the **reasoning**, not just the diff. The tutor evaluates the explanation
first, the code second, and treats missing reasoning as the thing to probe.

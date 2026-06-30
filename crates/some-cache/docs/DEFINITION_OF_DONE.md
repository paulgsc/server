# Definition of Done — `some-cache` M2

A story is **not** done because the code compiles or an agent produced a diff.
It is done when every box below is checked. These are deliverable requirements,
not aspirations.

## Per-story gates

A story (M2.x) is done when:

- [ ] Every `milestone_m2_x_*` test passes and its `#[ignore]` attribute is deleted.
- [ ] The `src/inproc` lint contract holds — `cargo clippy -p some-cache` reports
      **no new** findings in `inproc/*` (pre-existing crate debt outside `inproc`
      is tracked separately and is not a regression you introduced).
- [ ] `cargo fmt` clean (tabs; see `.rustfmt.toml`).
- [ ] The PR body contains the **reasoning** the story asks for (the conjecture),
      not just the diff. For M2.4 this includes the `# Cancellation` analysis.
- [ ] No `todo!()` / `unimplemented!()` remains in the files that story owns.

## Whole-milestone gates (M2 closes when all hold)

- [ ] `DedupCache` uses `some_cache::inproc::InProcCache`; the moka type is gone
      from `src/dedup.rs`.
- [ ] `moka` and `fastrand` are removed from `Cargo.toml`.
- [ ] `#![allow(dead_code)]` is removed from `src/inproc/mod.rs`.
- [ ] `cargo test -p some-cache -- --include-ignored` is green
      (zero-alloc tests run with `--test-threads=1`).
- [ ] `cargo tree -p some-cache` no longer lists the moka subtree
      (`crossbeam-epoch`, `tagptr`, `event-listener`, `quanta`, …).
- [ ] Behavior parity: the dedup/thundering-herd semantics observable to the two
      bins (axum server, pipeline daemon) are unchanged. `FETCH_DURATION` and
      `DEDUP_WAITERS` still record correctly.
- [ ] Exit evidence (per `docs/ROADMAP.md` M2.5) is attached to the closing PR:
      before/after `cargo tree`, false-sharing benchmark numbers, zero-alloc
      proof, and a short retrospective paragraph.

## Quality bar (the mechanical-sympathy axis)

- [ ] Steady-state `get`/`insert` and `probability_gate` allocate **zero** times
      (proven, not asserted).
- [ ] `Shard` is ≥64-byte aligned; the padded-vs-unpadded benchmark shows the
      false-sharing effect.
- [ ] Capacity is a hard bound under churn (CLOCK never grows past capacity).
- [ ] Single-flight: exactly one fetch under contention; cancellation-safe.
- [ ] Shard selection uses a power-of-two mask, not `%` (no division on the hot path).

## Anti-goals (explicitly out of scope for M2)

- Reworking the Redis `CacheStore` layer, compression, or the metrics design.
- Touching other crates (the HTML/regex parsers, ws-connection, etc.).
- Swapping moka for a different third-party cache crate (that defeats both the
  dependency-hygiene and the learning objective — see ADR-001 Alternative B).

//! Milestone integration tests — the definitive source of truth for sprint
//! progress on the "mechanical sympathy" cache refactor (GitHub milestone 2).
//!
//! A milestone is complete when ALL of its tests pass — here AND the zero-alloc
//! half in `alloc_guard.rs` — with the `inproc` lint contract intact and
//! `cargo clippy -p some-cache` clean for the module.
//!
//! Structural tests (type/layout invariants) run today and should already be
//! green — they encode constraints the skeleton already enforces. Behavioral
//! tests are `#[ignore]`d while bodies are `todo!()`; delete each `#[ignore]`
//! as you implement its milestone.
//!
//!   Run everything (incl. ignored):  cargo test -p some-cache -- --include-ignored
//!   Run one milestone:               cargo test -p some-cache milestone_m2_3
//!
//! See `docs/ROADMAP.md` for the full milestone definitions and
//! `docs/DEFINITION_OF_DONE.md` for the acceptance bar.

// ============================================================================
// M2.0 — Drop `fastrand`: thread-local probability gate
// ============================================================================

mod milestone_m2_0 {
	use some_cache::inproc::rng::probability_gate;

	/// The boundary contract holds regardless of RNG internals and needs no
	/// implementation — these branches short-circuit before touching the PRNG.
	#[test]
	fn milestone_m2_0_boundaries_are_deterministic() {
		assert!(!probability_gate(0.0), "p<=0 must never fire");
		assert!(!probability_gate(-1.0), "negative p must never fire");
		assert!(probability_gate(1.0), "p>=1 must always fire");
		assert!(probability_gate(2.0), "p>1 must always fire");
		println!("✅ M2.0: probability gate boundary contract");
	}

	/// Statistical sanity: ~p of N trials fire. Needs the real implementation.
	#[test]
	#[ignore = "M2.0: implement next_u64()/seed(), then delete this #[ignore]"]
	fn milestone_m2_0_is_approximately_uniform() {
		const N: u32 = 100_000;
		let hits = (0..N).filter(|_| probability_gate(0.25)).count() as f64;
		let rate = hits / f64::from(N);
		assert!((rate - 0.25).abs() < 0.02, "expected ~0.25, got {rate:.4}");
		println!("✅ M2.0: gate is statistically uniform (rate = {rate:.4})");
	}
}

// ============================================================================
// M2.1 — Define the `InProcStore` trait (the minimal moka surface)
// ============================================================================

mod milestone_m2_1 {
	use some_cache::inproc::{InProcCache, InProcStore};

	/// Compile-time proof that `InProcCache` implements the trait and the trait
	/// is object-safe. If this fails to compile, M2.1's surface is wrong.
	#[test]
	fn milestone_m2_1_trait_surface_is_object_safe() {
		fn assert_impls<T: InProcStore>() {}
		assert_impls::<InProcCache>();

		// Object safety: a `dyn InProcStore` must be nameable.
		fn _takes_dyn(_c: &dyn InProcStore) {}
		println!("✅ M2.1: InProcCache: InProcStore and the trait is object-safe");
	}
}

// ============================================================================
// M2.2 — Sharded storage, cache-line padded (false-sharing avoidance)
// ============================================================================

mod milestone_m2_2 {
	use some_cache::inproc::shard::Shard;

	/// The whole point of M2.2: each shard occupies its own cache line so locks
	/// in adjacent shards never false-share. This is a layout invariant the
	/// skeleton already encodes via `#[repr(align(64))]` — it should be green now.
	#[test]
	fn milestone_m2_2_shard_is_cache_line_aligned() {
		let align = std::mem::align_of::<Shard>();
		assert!(align >= 64, "Shard must be >=64-byte aligned to avoid false sharing, got {align}");
		println!("✅ M2.2: Shard is cache-line aligned ({align} bytes)");
	}

	/// Behavioral: a key always routes to the same shard, and the cache spreads
	/// keys across more than one shard. Needs `with_capacity`/`shard_for`.
	#[test]
	#[ignore = "M2.2: implement with_capacity + shard_for, then delete this #[ignore]"]
	fn milestone_m2_2_keys_distribute_across_shards() {
		// See docs/ROADMAP.md M2.2 for the exact distribution assertion to write
		// once `shard_for` is exposed to tests (or assert via observable behavior).
		println!("✅ M2.2: key distribution — implement per ROADMAP");
	}
}

// ============================================================================
// M2.3 — Bounded CLOCK eviction
// ============================================================================

mod milestone_m2_3 {
	use some_cache::inproc::lru::LruMap;
	use std::sync::Arc;

	#[test]
	#[ignore = "M2.3: implement LruMap, then delete this #[ignore]"]
	fn milestone_m2_3_get_insert_roundtrip() {
		let mut map = LruMap::new(4);
		let v: Arc<[u8]> = Arc::from(&b"x"[..]);
		map.insert(1, Arc::clone(&v));
		assert_eq!(map.get(1).as_deref(), Some(&b"x"[..]));
		assert_eq!(map.len(), 1);
		println!("✅ M2.3: get/insert roundtrip");
	}

	#[test]
	#[ignore = "M2.3: implement LruMap eviction, then delete this #[ignore]"]
	fn milestone_m2_3_never_exceeds_capacity() {
		let mut map = LruMap::new(4);
		let v: Arc<[u8]> = Arc::from(&b"x"[..]);
		for k in 0..100_u64 {
			map.insert(k, Arc::clone(&v));
			assert!(map.len() <= 4, "len {} exceeded capacity 4", map.len());
		}
		assert_eq!(map.len(), 4, "a full CLOCK map stays exactly at capacity");
		println!("✅ M2.3: bounded — never exceeds capacity under churn");
	}

	#[test]
	#[ignore = "M2.3: implement LruMap, then delete this #[ignore]"]
	fn milestone_m2_3_referenced_entry_survives_eviction() {
		// CLOCK second-chance: an entry touched via get() should outlive a cold
		// entry when the hand sweeps. Exact sequence in docs/ROADMAP.md M2.3.
		let mut map = LruMap::new(2);
		let v: Arc<[u8]> = Arc::from(&b"x"[..]);
		map.insert(1, Arc::clone(&v));
		map.insert(2, Arc::clone(&v));
		let _ = map.get(1); // give key 1 a second chance
		map.insert(3, Arc::clone(&v)); // should evict 2, not 1
		assert!(map.get(1).is_some(), "referenced entry must survive");
		println!("✅ M2.3: CLOCK second-chance protects referenced entries");
	}
}

// ============================================================================
// M2.4 — Single-flight (thundering-herd guard) + cancellation safety
// ============================================================================

mod milestone_m2_4 {
	use some_cache::inproc::single_flight::SingleFlight;

	/// Structural anchor: the registry exists and starts empty. The real
	/// behavioral tests (one fetch under N concurrent callers; leader
	/// cancellation does not hang followers) require a tokio runtime — add
	/// `tokio` dev-dependency features (`rt-multi-thread`, `macros`) in M2.4 and
	/// port the assertions from docs/ROADMAP.md M2.4 here.
	#[test]
	#[ignore = "M2.4: implement single-flight; see ROADMAP for the concurrency + cancellation tests"]
	fn milestone_m2_4_registry_starts_empty() {
		let sf = SingleFlight::new();
		assert_eq!(sf.outstanding(), 0);
		println!("✅ M2.4: single-flight registry starts empty (port concurrency tests per ROADMAP)");
	}
}

// ============================================================================
// M2.5 — Cutover: DedupCache uses InProcCache; moka + fastrand deleted
// ============================================================================

mod milestone_m2_5 {
	/// The cutover gate: no `moka` / `fastrand` in the manifest. `include_str!`
	/// embeds the manifest at compile time, so this goes green only after the
	/// dependencies are actually removed and the crate rebuilt.
	#[test]
	#[ignore = "M2.5: remove moka + fastrand from Cargo.toml, then delete this #[ignore]"]
	fn milestone_m2_5_dependencies_removed() {
		let manifest = include_str!("../Cargo.toml");
		assert!(!manifest.contains("moka"), "moka must be gone from Cargo.toml at cutover");
		assert!(!manifest.contains("fastrand"), "fastrand must be gone from Cargo.toml at cutover");
		println!("✅ M2.5: moka + fastrand removed from the dependency tree");
	}
}

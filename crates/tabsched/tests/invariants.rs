/// Core invariant tests.
///
/// Each test probes exactly one invariant from the spec.
/// No IO, no Engine — tests call `next_session` / `apply` directly.
use std::{collections::HashMap, num::NonZeroU32};

use tabsched::{apply, next_session, record_outcome, Resource, ResourceId, State, Topology, Track, TrackId};

// ── Topology fixture ───────────────────────────────────────────────────

fn nz(n: u32) -> NonZeroU32 {
	NonZeroU32::new(n).unwrap()
}

/// Build a simple two-level tree:
fn fixture() -> Topology {
	let tracks = vec![
		Track::internal(TrackId(0), None, "root", nz(10), vec![TrackId(1), TrackId(2), TrackId(3)]).unwrap(),
		Track::leaf(TrackId(1), Some(TrackId(0)), "dsa", nz(5), vec![ResourceId(0), ResourceId(1), ResourceId(2)]).unwrap(),
		Track::leaf(TrackId(2), Some(TrackId(0)), "math", nz(3), vec![ResourceId(3), ResourceId(4)]).unwrap(),
		Track::leaf(TrackId(3), Some(TrackId(0)), "lang", nz(2), vec![ResourceId(5)]).unwrap(),
	];

	let resources = (0u32..=5).map(|i| Resource::new(ResourceId(i), format!("r{i}"))).collect();

	Topology::new(tracks, resources).unwrap()
}

// ── I1: Determinism ────────────────────────────────────────────────────

#[test]
fn i1_same_state_same_session() {
	let topo = fixture();
	let state = State::new(&topo, 10);

	let s1 = next_session(&state, &topo);
	let s2 = next_session(&state, &topo);

	assert_eq!(s1, s2, "next_session must be deterministic");
}

// ── I2: One slot → one session ─────────────────────────────────────────

#[test]
fn i2_apply_increments_history_by_one() {
	let topo = fixture();
	let s0 = State::new(&topo, 10);

	let session = next_session(&s0, &topo);
	let s1 = apply(&s0, &session);

	assert_eq!(s1.history.len(), s0.history.len() + 1);
}

// ── I3: Fairness over window ───────────────────────────────────────────

#[test]
fn i3_fairness_over_window() {
	let topo = fixture();
	let window_size = 10;
	let epsilon = 2i32;
	let mut state = State::new(&topo, window_size);

	for _ in 0..100 {
		let session = next_session(&state, &topo);
		state = apply(&state, &session);
	}

	let tail = &state.history[state.history.len() - window_size..];

	let mut counts: HashMap<TrackId, u32> = HashMap::new();
	for s in tail {
		*counts.entry(s.track).or_default() += 1;
	}

	let targets = [(TrackId(1), 5u32), (TrackId(2), 3), (TrackId(3), 2)];

	for (tid, target) in targets {
		let actual = counts.get(&tid).copied().unwrap_or(0) as i32;
		let diff = (actual - target as i32).abs();

		assert!(diff <= epsilon, "track {tid}: target={target}, actual={actual}, diff={diff} > ε={epsilon}");
	}
}

// ── I5: Cyclic resource visitation ─────────────────────────────────────

#[test]
fn i5_cyclic_resource_order() {
	let tracks = vec![
		Track::internal(TrackId(0), None, "root", nz(6), vec![TrackId(1)]).unwrap(),
		Track::leaf(TrackId(1), Some(TrackId(0)), "dsa", nz(6), vec![ResourceId(0), ResourceId(1), ResourceId(2)]).unwrap(),
	];

	let resources = (0u32..3).map(|i| Resource::new(ResourceId(i), format!("r{i}"))).collect();

	let topo = Topology::new(tracks, resources).unwrap();
	let mut state = State::new(&topo, 100);

	let expected_cycle = [ResourceId(0), ResourceId(1), ResourceId(2)];

	for (i, expected) in expected_cycle.iter().cycle().take(9).enumerate() {
		let session = next_session(&state, &topo);

		assert_eq!(session.resource, *expected, "slot {i}: expected {expected}, got {}", session.resource);

		state = apply(&state, &session);
	}
}

// ── I4: Hierarchical conservation ──────────────────────────────────────

#[test]
fn i4_parent_count_equals_leaf_sum() {
	let topo = fixture();
	let mut state = State::new(&topo, 20);

	for _ in 0..50 {
		let s = next_session(&state, &topo);
		state = apply(&state, &s);
	}

	let leaf_total = [TrackId(1), TrackId(2), TrackId(3)]
		.iter()
		.map(|&tid| state.history.iter().filter(|s| s.track == tid).count())
		.sum::<usize>();

	assert_eq!(state.history.len(), leaf_total, "total sessions must equal sum of leaf sessions");
}

// ── I6: Outcome irrelevance ────────────────────────────────────────────

#[test]
fn i6_outcome_does_not_affect_scheduling() {
	use tabsched::Outcome;

	let topo = fixture();
	let s0 = State::new(&topo, 10);

	let session = next_session(&s0, &topo);
	let s1 = apply(&s0, &session);

	let s1_stuck = record_outcome(&s1, Outcome::Stuck);
	let s1_progress = record_outcome(&s1, Outcome::Progress);

	let next_stuck = next_session(&s1_stuck, &topo);
	let next_progress = next_session(&s1_progress, &topo);

	assert_eq!(next_stuck, next_progress, "different outcomes must not change next session selection");
}

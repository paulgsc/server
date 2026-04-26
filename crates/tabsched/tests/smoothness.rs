/// Smoothness and anti-starvation tests.
///
/// - I7:  No two consecutive picks of the same leaf track.
/// - I8:  Low-weight tracks do not experience extended droughts.
/// - I9:  Recency penalty decays.
/// - I10: Branches in a multi-level tree do not starve each other.
///
/// I10 is the primary regression test for the SubtreeWindow fix.
use std::{collections::HashMap, num::NonZeroU32};

use tabsched::{apply, next_session, Resource, ResourceId, State, Topology, Track, TrackId};

fn nz(n: u32) -> NonZeroU32 {
	NonZeroU32::new(n).unwrap()
}

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

fn skewed_fixture() -> Topology {
	let tracks = vec![
		Track::internal(TrackId(0), None, "root", nz(10), vec![TrackId(1), TrackId(2)]).unwrap(),
		Track::leaf(TrackId(1), Some(TrackId(0)), "heavy", nz(9), vec![ResourceId(0), ResourceId(1)]).unwrap(),
		Track::leaf(TrackId(2), Some(TrackId(0)), "light", nz(1), vec![ResourceId(2)]).unwrap(),
	];
	let resources = (0u32..3).map(|i| Resource::new(ResourceId(i), format!("r{i}"))).collect();
	Topology::new(tracks, resources).unwrap()
}

/// Two-level tree with two internal branches.
///
/// root (20)
/// ├── rust-branch  (12) TrackId(1)  ← lower id, always won old tie-break
/// │   ├── rust-internals (8)  TrackId(3)
/// │   └── rust-systems   (4)  TrackId(4)
/// └── other-branch  (8) TrackId(2)  ← starved before SubtreeWindow fix
///     ├── korean (5) TrackId(5)
///     └── math   (3) TrackId(6)
fn two_branch_fixture() -> Topology {
	let tracks = vec![
		Track::internal(TrackId(0), None, "root", nz(20), vec![TrackId(1), TrackId(2)]).unwrap(),
		Track::internal(TrackId(1), Some(TrackId(0)), "rust-branch", nz(12), vec![TrackId(3), TrackId(4)]).unwrap(),
		Track::leaf(TrackId(3), Some(TrackId(1)), "rust-internals", nz(8), vec![ResourceId(0), ResourceId(1)]).unwrap(),
		Track::leaf(TrackId(4), Some(TrackId(1)), "rust-systems", nz(4), vec![ResourceId(2), ResourceId(3)]).unwrap(),
		Track::internal(TrackId(2), Some(TrackId(0)), "other-branch", nz(8), vec![TrackId(5), TrackId(6)]).unwrap(),
		Track::leaf(TrackId(5), Some(TrackId(2)), "korean", nz(5), vec![ResourceId(4), ResourceId(5)]).unwrap(),
		Track::leaf(TrackId(6), Some(TrackId(2)), "math", nz(3), vec![ResourceId(6), ResourceId(7)]).unwrap(),
	];
	let resources = (0u32..8).map(|i| Resource::new(ResourceId(i), format!("r{i}"))).collect();
	Topology::new(tracks, resources).unwrap()
}

// ── I7: No consecutive picks ───────────────────────────────────────────────

#[test]
fn i7_no_consecutive_picks() {
	let topo = fixture();
	let mut state = State::new(&topo, 10);
	let mut last_track = None;

	for slot in 0..200 {
		let session = next_session(&state, &topo);
		if let Some(prev) = last_track {
			assert_ne!(session.track, prev, "slot {slot}: consecutive pick of {prev}");
		}
		last_track = Some(session.track);
		state = apply(&state, &session);
	}
}

// ── I8: Low-weight track drought bound ────────────────────────────────────

#[test]
fn i8_low_weight_track_max_drought() {
	let topo = skewed_fixture();
	let mut state = State::new(&topo, 20);
	let light = TrackId(2);
	let max_gap = 15usize;
	let mut last_light_slot = None::<usize>;

	for slot in 0..300 {
		let session = next_session(&state, &topo);
		state = apply(&state, &session);

		if session.track == light {
			if let Some(prev) = last_light_slot {
				let gap = slot - prev;
				assert!(gap <= max_gap, "slot {slot}: drought = {gap} > {max_gap}");
			}
			last_light_slot = Some(slot);
		}
	}
	assert!(last_light_slot.is_some(), "light track never selected — starvation");
}

// ── I9: Recency penalty decays ─────────────────────────────────────────────

#[test]
fn i9_recency_penalty_decays() {
	let topo = fixture();
	let mut state = State::new(&topo, 20);
	for _ in 0..20 {
		let s = next_session(&state, &topo);
		state = apply(&state, &s);
	}

	let mut last_served: HashMap<TrackId, usize> = HashMap::new();
	let mut max_gap_seen: HashMap<TrackId, usize> = HashMap::new();

	for slot in 0..100usize {
		let session = next_session(&state, &topo);
		state = apply(&state, &session);
		let tid = session.track;
		if let Some(prev) = last_served.insert(tid, slot) {
			let entry = max_gap_seen.entry(tid).or_insert(0);
			*entry = (*entry).max(slot - prev);
		}
	}

	let targets = [(TrackId(1), 5u32), (TrackId(2), 3), (TrackId(3), 2)];
	for (tid, target) in targets {
		if let Some(&gap) = max_gap_seen.get(&tid) {
			let bound = 2 * (20u32 / target) as usize + 2;
			assert!(gap <= bound, "track {tid}: gap={gap} > bound={bound}");
		}
	}
}

// ── I10: Branch fairness — SubtreeWindow regression ────────────────────────

/// Both internal branches must receive sessions in proportion to their
/// `base_target`.  Before `SubtreeWindow`, `other-branch` (TrackId=2)
/// received zero sessions because internal-node window counts were always
/// 0 and tie-breaking by TrackId always chose rust-branch (TrackId=1).
#[test]
fn i10_branch_fairness() {
	let topo = two_branch_fixture();
	let mut state = State::new(&topo, 20);
	let total = 200usize;
	for _ in 0..total {
		let s = next_session(&state, &topo);
		state = apply(&state, &s);
	}

	let rust_leaves = [TrackId(3), TrackId(4)];
	let other_leaves = [TrackId(5), TrackId(6)];

	let rust_count = state.history.iter().filter(|s| rust_leaves.contains(&s.track)).count();
	let other_count = state.history.iter().filter(|s| other_leaves.contains(&s.track)).count();

	// rust-branch:other-branch = 12:8 = 3:2
	let rust_expected = total * 12 / 20; // 120
	let other_expected = total * 8 / 20; //  80
	let epsilon = total / 10; //  20

	assert!(rust_count.abs_diff(rust_expected) <= epsilon, "rust-branch: expected ~{rust_expected}, got {rust_count}");
	assert!(other_count > 0, "other-branch got 0 sessions — SubtreeWindow fix missing or broken");
	assert!(
		other_count.abs_diff(other_expected) <= epsilon,
		"other-branch: expected ~{other_expected}, got {other_count} — branch starvation"
	);
}

/// Within the previously-starved branch, leaves must also be fair.
#[test]
fn i10b_starved_branch_internal_fairness() {
	let topo = two_branch_fixture();
	let mut state = State::new(&topo, 20);
	for _ in 0..400 {
		let s = next_session(&state, &topo);
		state = apply(&state, &s);
	}

	let k = state.history.iter().filter(|s| s.track == TrackId(5)).count() as f64;
	let m = state.history.iter().filter(|s| s.track == TrackId(6)).count() as f64;

	assert!(k > 0.0 && m > 0.0, "leaves inside other-branch never reached");
	let ratio = k / m; // expected ~5/3 ≈ 1.67
	assert!((1.2..=2.2).contains(&ratio), "korean/math ratio={ratio:.2}, want ~1.67");
}

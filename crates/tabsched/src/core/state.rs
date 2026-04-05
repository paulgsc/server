use super::{
	cursor::CursorMap,
	deficit::Adjustments,
	select::{next_selection, Selection},
	window::RingWindow,
	window::WindowCounter,
};
use crate::domain::{
	ids::SlotIndex,
	session::{Outcome, Session},
	topology::Topology,
};

/// The complete, clonable scheduler state.
///
/// # Immutability contract
///
/// `State` is a pure value type. `next_session` and `apply` take `&State`
/// and return new values — they do not mutate in place.
///
/// The `runtime::Engine` wrapper is the single site where `&mut` appears;
/// it owns a `State` and advances it one slot at a time. This separation
/// means the core logic is testable without any mutable borrows and can
/// be replayed deterministically from any snapshot.
///
/// # Window implementation
///
/// `State` owns a `RingWindow` for O(1) count queries. It is always
/// consistent with `history` — both are updated atomically in `apply`.
/// If you need the `SlidingWindow` variant (simpler, for testing), use
/// `next_session_with_window` directly.
#[derive(Debug, Clone)]
pub struct State {
	pub history: Vec<Session>,
	pub cursors: CursorMap,
	pub adjustments: Adjustments,
	window: RingWindow,
	window_size: usize,
}

impl State {
	pub fn new(topology: &Topology, window_size: usize) -> Self {
		Self {
			history: Vec::new(),
			cursors: CursorMap::new(topology),
			adjustments: Adjustments::default(),
			window: RingWindow::new(window_size),
			window_size,
		}
	}

	pub fn window_size(&self) -> usize {
		self.window_size
	}

	/// Restore a `State` from a persisted history. Rehydrates the ring
	/// window and cursor map from scratch.
	pub fn from_history(history: Vec<Session>, topology: &Topology, window_size: usize) -> Self {
		let window = RingWindow::from_history(&history, window_size);
		let mut cursors = CursorMap::new(topology);
		// replay cursors from history
		for s in &history {
			cursors = cursors.advance(s.track);
		}
		Self {
			history,
			cursors,
			adjustments: Adjustments::default(),
			window,
			window_size,
		}
	}
}

// ── Pure functions ─────────────────────────────────────────────────────────

/// Determine the next session **without** modifying any state.
///
/// This is the primary pure entry point. Call `apply` to produce the
/// new state after the session is confirmed.
pub fn next_session(state: &State, topology: &Topology) -> Session {
	next_session_with_window(state, topology, &state.window)
}

/// Variant that accepts an arbitrary `WindowCounter` — useful for
/// swapping in a `SlidingWindow` in tests.
pub fn next_session_with_window<W: WindowCounter>(state: &State, topology: &Topology, window: &W) -> Session {
	let Selection { leaf, resource } = next_selection(topology, &state.cursors, window, &state.adjustments);

	Session {
		slot_index: SlotIndex(state.history.len() as u64),
		track: leaf,
		resource,
		outcome: Outcome::Unrecorded,
	}
}

/// Produce a new `State` that reflects `session` having been executed.
///
/// `session` should be the value returned by `next_session` (same state).
/// Calling `apply` with a fabricated session is allowed but breaks the
/// fairness invariant — the ring window and cursor will diverge.
pub fn apply(state: &State, session: &Session) -> State {
	let mut new_state = state.clone();
	new_state.cursors = new_state.cursors.advance(session.track);
	new_state.window.push(session.track);
	new_state.history.push(session.clone());
	new_state
}

/// Convenience: apply a completed outcome to the most recent session.
///
/// Outcome is stored for analysis but **never** read by the scheduler.
pub fn record_outcome(state: &State, outcome: Outcome) -> State {
	let mut new_state = state.clone();
	if let Some(last) = new_state.history.last_mut() {
		last.outcome = outcome;
	}
	new_state
}

// ── Serialisation accessor ─────────────────────────────────────────────────

impl State {
	/// Raw cursor positions for all leaf tracks visited so far.
	/// Used exclusively by the persistence adapter — not by scheduling logic.
	pub fn cursors_raw(&self) -> impl Iterator<Item = (crate::domain::ids::TrackId, u64)> + '_ {
		// Collect distinct leaf track ids from history, then look up
		// each cursor value. This avoids exposing CursorMap internals.
		let seen: std::collections::BTreeSet<crate::domain::ids::TrackId> = self.history.iter().map(|s| s.track).collect();
		seen.into_iter().filter_map(|tid| self.cursors.get(tid).map(|pos| (tid, pos)))
	}
}

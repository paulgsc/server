use crate::core::state::{apply, next_session, record_outcome, State};
use crate::domain::{session::Outcome, session::Session, topology::Topology};

/// The single site where `&mut` appears.
///
/// `Engine` owns a `State` and a reference to the (immutable) `Topology`.
/// All mutation is gated through `step()` and `record_outcome()`.
///
/// # Relationship to actor pattern
///
/// The discussion deferred the actor model until IO/concurrency is needed,
/// which is the right call. However, `Engine` is deliberately structured
/// so that wrapping it in an actor (e.g. a `tokio` task with a command
/// channel) requires zero changes to the core — you would write:
///
/// ```ignore
/// struct SchedulerActor {
///     engine: Engine,
///     rx: mpsc::Receiver<Command>,
/// }
/// ```
///
/// and forward `step` / `record_outcome` calls from the channel loop.
/// No core logic moves.
///
/// # Thread safety
///
/// `Engine` is `Send` but not `Sync` — it holds `&mut` internally. If
/// you need concurrent read access to `State`, clone the state snapshot
/// and query it separately; queries never mutate.
pub struct Engine<'topo> {
	state: State,
	topology: &'topo Topology,
}

impl<'topo> Engine<'topo> {
	pub fn new(state: State, topology: &'topo Topology) -> Self {
		Self { state, topology }
	}

	/// Advance one slot:
	/// 1. Compute the next session (pure).
	/// 2. Apply it to produce the new state.
	/// 3. Return the session so the caller can display/act on it.
	///
	/// This is the **only** public mutation point.
	pub fn step(&mut self) -> Session {
		let session = next_session(&self.state, self.topology);
		self.state = apply(&self.state, &session);
		session
	}

	/// Attach an outcome to the most recently completed session.
	///
	/// Idempotent on empty history (no-op).
	pub fn record_outcome(&mut self, outcome: Outcome) {
		self.state = record_outcome(&self.state, outcome);
	}

	/// Read-only snapshot of the current state.
	///
	/// Callers may clone this for persistence, analysis, or testing
	/// without affecting the engine.
	pub fn state(&self) -> &State {
		&self.state
	}

	/// Consume the engine, returning the final state.
	pub fn into_state(self) -> State {
		self.state
	}
}

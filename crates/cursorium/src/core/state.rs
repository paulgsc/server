use smallvec::SmallVec;

use super::Cursor;
use super::{ActiveLifetime, LifetimeEvent, OrchestratorEvent, OrchestratorState, Progress, TimeMs, TimedEvent, Timeline};

/// Internal mutable state (owned by engine actor)
///
/// # Concurrency Model
///
/// This state machine supports **concurrent lifetimes** - multiple scenes, components,
/// or other entities can be active simultaneously with overlapping time windows.
///
/// ## Key Design Invariants
///
/// 1. **Events are facts, not commands**
///    - Each `TimedEvent` carries its own timestamp (`at`)
///    - Events are applied at their declared time, not "current" time
///    - This enables deterministic replay via `reconstruct_from_start`
///
/// 2. **Lifetimes are intervals, not scalar state**
///    - `active_lifetimes` is a **set** (SmallVec), not a single value
///    - Multiple scenes can be active concurrently during transitions
///    - Clients must track lifetimes by ID, not assume single "current scene"
///
/// 3. **Time is derived, not authoritative**
///    - `calculate_current_time()` derives time from system clock
///    - State reconstruction uses event timestamps as source of truth
///
/// 4. **Mode represents lifecycle state**
///    - `mode` is the orchestrator's FSM state (Unconfigured, Idle, Running, etc.)
///    - Mode transitions are explicit and controlled by command handlers
///    - Terminal modes (Finished, Stopped, Error) require cleanup
///
/// ## Event Ordering Considerations
///
/// **Current Limitation**: Events at the same timestamp (`at`) have no guaranteed order.
///
/// This is acceptable for most cases but creates ambiguity when:
/// - Multiple lifetimes start at exactly the same tick
/// - Event replay order affects final state
/// - Debugging requires precise causal ordering
///
/// ### Recommended Future Enhancement
///
/// Add a monotonic sequence number to `TimedEvent`:
///
/// ```rust,ignore
/// pub struct TimedEvent<E> {
///     pub at: TimeMs,
///     pub seq: u64,        // Total order guarantee
///     pub event: E,
/// }
/// ```
///
/// **Benefits**:
/// - Deterministic replay across all edge cases
/// - Causal ordering visible in logs/debugging
/// - Enables event sourcing patterns
/// - Nearly zero cost (single counter increment per event)
///
/// **When to add**: Before implementing undo/redo, distributed replay, or
/// when debugging non-deterministic state reconstruction issues.
///
/// ## Client Integration Notes
///
/// Clients receiving `OrchestratorState` should:
/// - Track `active_lifetimes` as a **set**, indexed by `id`
/// - Detect scene changes via **set difference**, not scalar comparison
/// - Use `current_active_scene` only as a UI hint, not structural truth
/// - Check `mode` to determine if orchestrator is active, terminal, etc.
///
/// Example (TypeScript):
/// ```typescript,ignore
/// // ❌ Wrong: assumes single scene
/// const sceneChanged = prev.scene_id !== next.scene_id
///
/// // ✅ Correct: handles concurrent scenes
/// const prevScenes = new Set(extractSceneIds(prev.lifetimes))
/// const nextScenes = new Set(extractSceneIds(next.lifetimes))
/// const sceneChanged = !setEquals(prevScenes, nextScenes)
///
/// // Check mode instead of boolean flags
/// const isActive = next.mode === "Running"
/// const needsCleanup = ["Finished", "Stopped", "Error"].includes(next.mode)
/// ```
pub(crate) struct EngineState {
	// Observable state
	pub state: OrchestratorState,

	// Time tracking
	pub start_instant: Option<std::time::Instant>,
	pub paused_at: Option<TimeMs>,
	pub accumulated_pause_duration: TimeMs,

	// Zero-allocation buffers for active lifetimes
	pub active_lifetimes: SmallVec<[ActiveLifetime; 8]>,
}

impl EngineState {
	pub fn new(total_duration: TimeMs) -> Self {
		Self {
			state: OrchestratorState::new(total_duration),
			start_instant: None,
			paused_at: None,
			accumulated_pause_duration: 0,
			active_lifetimes: SmallVec::new(),
		}
	}

	/// Calculate elapsed time since orchestration start, accounting for pauses
	///
	/// Returns wall-clock time minus accumulated pause duration.
	/// Returns 0 if orchestration hasn't started.
	pub fn calculate_current_time(&self) -> TimeMs {
		if let Some(start) = self.start_instant {
			let elapsed = start.elapsed().as_millis() as TimeMs;
			elapsed.saturating_sub(self.accumulated_pause_duration)
		} else {
			0
		}
	}

	/// Apply an event to the state (pure reducer)
	///
	/// # Event Semantics
	///
	/// - Events are applied at their own timestamp (`event.at`)
	/// - Time is a **fact** embedded in the event, not a parameter
	/// - This design enables deterministic replay and event sourcing
	///
	/// # Concurrency Handling
	///
	/// Multiple lifetimes can be active simultaneously:
	/// - `Start` events push to `active_lifetimes` (set semantics)
	/// - `End` events remove by ID
	/// - No enforcement of mutual exclusion between lifetimes
	///
	/// ## Note on Event Ordering
	///
	/// Events at the same timestamp are processed in the order received,
	/// but this order is **not guaranteed** to be stable across replays.
	///
	/// For guaranteed replay determinism, consider adding a sequence number
	/// to `TimedEvent` (see struct-level docs).
	pub fn apply_event(&mut self, event: &TimedEvent<OrchestratorEvent>) {
		match &event.event {
			OrchestratorEvent::Lifetime(lifetime_event) => match lifetime_event {
				LifetimeEvent::Start { id, kind } => {
					self.active_lifetimes.push(ActiveLifetime {
						id: *id,
						kind: kind.clone(),
						started_at: event.at,
					});
				}
				LifetimeEvent::End { id } => {
					self.active_lifetimes.retain(|l| l.id != *id);
				}
			},
			OrchestratorEvent::Point(_point) => {
				// Point events can be emitted to observers or logged
				// They don't accumulate in state (avoid unbounded growth)
			}
		}
	}

	/// Sync observable state after events have been applied
	///
	/// Updates all time-derived presentation fields and copies internal
	/// lifetime tracking to the observable `OrchestratorState`.
	///
	/// # Concurrency Implications
	///
	/// - `active_lifetimes` may contain multiple scene lifetimes
	/// - `current_active_scene` is a **projection** (first scene found)
	/// - Clients should NOT rely on `current_active_scene` as structural truth
	///
	/// ## Migration Recommendation
	///
	/// Consider deprecating `current_active_scene` in favor of having clients
	/// query `active_lifetimes` directly. This makes concurrency explicit and
	/// prevents incorrect assumptions about single-scene semantics.
	pub fn sync_view_state(&mut self, current_time: TimeMs) {
		// Update time-derived presentation state
		self.state.current_time = current_time;
		self.state.progress = Progress::new(current_time, self.state.total_duration);
		self.state.time_remaining = self.state.total_duration.saturating_sub(current_time);

		// Sync active lifetimes to observable state
		self.state.active_lifetimes = self.active_lifetimes.iter().cloned().collect();

		// Update current active scene (first active scene lifetime)
		// NOTE: This is a lossy projection when multiple scenes are concurrent.
		// Clients should use `active_lifetimes` for accurate concurrent tracking.
		self.state.current_active_scene = self.active_lifetimes.iter().find_map(|l| l.scene_name().map(String::from));
	}

	/// Reconstruct state by replaying events from the beginning
	///
	/// # Determinism Properties
	///
	/// This function provides **event-time determinism**: given the same timeline
	/// and target time, the resulting state is reproducible.
	///
	/// However, events at identical timestamps may be applied in different orders
	/// across runs. For **total determinism**, add sequence numbers to events.
	///
	/// # Use Cases
	///
	/// - Seeking to arbitrary timeline positions
	/// - Recovering from state corruption
	/// - Implementing undo/redo
	/// - Time-travel debugging
	///
	/// # Performance
	///
	/// O(n) where n = number of events before `time`. For frequent seeks,
	/// consider implementing checkpoint snapshots.
	pub fn reconstruct_from_start(&mut self, cursor: &mut Cursor, timeline: &Timeline, time: TimeMs) {
		self.active_lifetimes.clear();
		cursor.reset();

		cursor.apply_until(timeline, time, |event| {
			self.apply_event(event);
		});

		self.sync_view_state(time);
		self.start_instant = Some(std::time::Instant::now() - std::time::Duration::from_millis(time as u64));
		self.accumulated_pause_duration = 0;
		self.paused_at = None;
	}
}

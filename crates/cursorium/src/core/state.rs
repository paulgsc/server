use smallvec::SmallVec;

use super::Cursor;
use super::{ActiveLifetime, LifetimeEvent, OrchestratorEvent, OrchestratorState, Progress, TimeMs, TimedEvent, Timeline};

/// Internal mutable state (owned by engine actor)
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

	pub fn calculate_current_time(&self) -> TimeMs {
		if let Some(start) = self.start_instant {
			let elapsed = start.elapsed().as_millis() as TimeMs;
			elapsed.saturating_sub(self.accumulated_pause_duration)
		} else {
			0
		}
	}

	/// Apply an event to the state (pure reducer)
	/// Events are applied at their own timestamp - time is a fact, not an input
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

	/// Sync state after events have been applied
	pub fn sync_view_state(&mut self, current_time: TimeMs) {
		// Update time-derived presentation state
		self.state.current_time = current_time;
		self.state.progress = Progress::new(current_time, self.state.total_duration);
		self.state.time_remaining = self.state.total_duration.saturating_sub(current_time);

		// Sync active lifetimes to observable state
		self.state.active_lifetimes = self.active_lifetimes.iter().cloned().collect();

		// Update current active scene (first active scene lifetime)
		self.state.current_active_scene = self.active_lifetimes.iter().find_map(|l| l.scene_name().map(String::from));
	}

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

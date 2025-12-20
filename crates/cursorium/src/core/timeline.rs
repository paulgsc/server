use super::{OrchestratorEvent, TimeMs, TimedEvent};

/// An immutable, sorted timeline of events
#[derive(Debug, Clone)]
pub struct Timeline {
	events: Box<[TimedEvent<OrchestratorEvent>]>,
	total_duration: TimeMs,
}

impl Timeline {
	/// Create a new timeline from events (will be sorted)
	pub fn new(mut events: Vec<TimedEvent<OrchestratorEvent>>) -> Self {
		events.sort_by_key(|e| e.at);
		let total_duration = events.last().map(|e| e.at).unwrap_or(0);
		Self {
			events: events.into_boxed_slice(),
			total_duration,
		}
	}

	pub fn total_duration(&self) -> TimeMs {
		self.total_duration
	}

	pub fn is_empty(&self) -> bool {
		self.events.is_empty()
	}

	pub fn len(&self) -> usize {
		self.events.len()
	}

	pub fn events(&self) -> &[TimedEvent<OrchestratorEvent>] {
		&self.events
	}

	/// Create a cursor for this timeline
	pub fn cursor(&self) -> Cursor {
		Cursor::new()
	}
}

#[derive(Debug, Clone)]
pub struct Cursor {
	frontier: usize,
}

impl Cursor {
	pub fn new() -> Self {
		Self { frontier: 0 }
	}

	pub fn reset(&mut self) {
		self.frontier = 0;
	}

	pub fn applied_frontier(&self) -> usize {
		self.frontier
	}

	/// Apply events up to (and including) the given time
	/// Each event is applied exactly once - monotonic and irreversible
	///
	/// This is the ONLY method that advances the application frontier
	pub fn apply_until<F>(&mut self, timeline: &Timeline, time: TimeMs, mut on_event: F)
	where
		F: FnMut(&TimedEvent<OrchestratorEvent>),
	{
		let events = timeline.events();

		// Only apply events we haven't applied yet
		while self.frontier < events.len() {
			let event = &events[self.frontier];

			// Stop if we've reached events beyond target time
			if event.at > time {
				break;
			}

			on_event(event);
			self.frontier += 1;
		}
	}
}

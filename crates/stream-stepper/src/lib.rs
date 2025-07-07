pub mod error;
pub mod event;
pub mod state;
pub mod timeline;
pub mod types;

pub use error::{ChapterError, Result};
pub use event::TimelineEvent;
pub use state::{Chapter, TimelineState};
pub use timeline::LiveTimeline;
pub use types::*;

pub struct LiveChapters {
	timeline: LiveTimeline,
}

impl LiveChapters {
	pub fn new() -> Self {
		Self { timeline: LiveTimeline::new() }
	}

	pub fn process_event(&mut self, event: TimelineEvent) -> Result<TimelineState> {
		self.timeline.process_event(event)
	}

	pub fn current_state(&self) -> &TimelineState {
		self.timeline.current_state()
	}

	pub fn get_chapters_at(&self, timestamp: Timestamp) -> Vec<Chapter> {
		self.timeline.get_chapters_at(timestamp)
	}

	pub fn generate_time_series(&self, start: Timestamp, end: Timestamp, interval: u64) -> Vec<TimeSeriesPoint> {
		self.timeline.generate_time_series(start, end, interval)
	}
}

impl Default for LiveChapters {
	fn default() -> Self {
		Self::new()
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimeSeriesPoint {
	pub timestamp: Timestamp,
	pub chapters: Vec<Chapter>,
	pub active_count: usize,
	pub total_duration: u64,
}

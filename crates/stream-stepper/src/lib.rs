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

/// Main entry point for the Live Chapters system
pub struct LiveChapters {
	timeline: LiveTimeline,
}

impl LiveChapters {
	/// Create a new LiveChapters instance
	pub fn new() -> Self {
		Self { timeline: LiveTimeline::new() }
	}

	/// Process multiple events at time t and return the updated timeline snapshot
	pub fn process_events_at_time(&mut self, events: Vec<TimelineEvent>, current_time: Timestamp) -> Result<TimelineSnapshot> {
		// Process all events for this timestamp
		for event in events {
			self.timeline.process_event(event)?;
		}

		// Update timeline to current time
		self.timeline.advance_to(current_time);

		// Generate timeline snapshot for UI rendering
		self.timeline.generate_timeline_snapshot(current_time)
	}

	/// Process a single event and return the updated timeline snapshot
	pub fn process_event_at_time(&mut self, event: TimelineEvent, current_time: Timestamp) -> Result<TimelineSnapshot> {
		self.process_events_at_time(vec![event], current_time)
	}

	/// Get current timeline snapshot without processing events
	pub fn get_timeline_snapshot(&self, current_time: Timestamp) -> Result<TimelineSnapshot> {
		self.timeline.generate_timeline_snapshot(current_time)
	}

	/// Get the current state
	pub fn current_state(&self) -> &TimelineState {
		self.timeline.current_state()
	}
}

impl Default for LiveChapters {
	fn default() -> Self {
		Self::new()
	}
}

/// Timeline snapshot for UI rendering - represents the complete timeline at time t
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimelineSnapshot {
	/// The current livestream time
	pub current_time: Timestamp,
	/// Total duration of the livestream so far
	pub total_duration: u64,
	/// Ordered list of timeline segments for UI rendering
	pub segments: Vec<TimelineSegment>,
	/// Number of active (ongoing) chapters
	pub active_count: usize,
	/// State version for change tracking
	pub version: u64,
}

/// A segment in the timeline UI - represents a visual block in the stepper
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimelineSegment {
	/// Start time of this segment
	pub start_time: Timestamp,
	/// End time of this segment (None if still ongoing)
	pub end_time: Option<Timestamp>,
	/// Duration of this segment
	pub duration: u64,
	/// Title/topic of this segment
	pub title: String,
	/// Whether this segment is currently active
	pub is_active: bool,
	/// All chapters that overlap with this segment
	pub chapters: Vec<Chapter>,
	/// Percentage of total timeline this segment represents
	// TODO: Get rid of this
	pub percentage: f64,
}

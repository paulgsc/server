use crate::error::*;
use crate::event::TimelineEvent;
use crate::state::{Chapter, TimelineState};
use crate::types::*;
use crate::{TimelineSegment, TimelineSnapshot};
use std::collections::BTreeMap;

/// The main timeline processor that handles FSM transitions
pub struct LiveTimeline {
	state: TimelineState,
}

impl LiveTimeline {
	/// Create a new timeline
	pub fn new() -> Self {
		Self { state: TimelineState::new() }
	}

	/// Process an event and update state
	pub fn process_event(&mut self, event: TimelineEvent) -> Result<()> {
		match event {
			TimelineEvent::StartChapter {
				uid,
				context,
				start_time,
				payload,
			} => {
				self.handle_start_chapter(uid, context, start_time, payload)?;
			}

			TimelineEvent::EndChapter { uid, end_time, final_payload } => {
				self.handle_end_chapter(uid, end_time, final_payload)?;
			}

			TimelineEvent::UpdatePayload { uid, payload } => {
				self.handle_update_payload(uid, payload)?;
			}

			TimelineEvent::UpdateContext { uid, context } => {
				self.handle_update_context(uid, context)?;
			}

			TimelineEvent::RemoveChapter { uid } => {
				self.handle_remove_chapter(uid)?;
			}

			TimelineEvent::ExtendChapter { uid, extend_to } => {
				self.handle_extend_chapter(uid, extend_to)?;
			}

			TimelineEvent::CompleteChapter {
				uid,
				completion_time,
				final_payload,
			} => {
				self.handle_complete_chapter(uid, completion_time, final_payload)?;
			}

			TimelineEvent::ClearAll => {
				self.handle_clear_all();
			}
		}

		Ok(())
	}

	/// Advance timeline to current time
	pub fn advance_to(&mut self, current_time: Timestamp) {
		self.state.update_current_time(current_time);
	}

	/// Get the current state
	pub fn current_state(&self) -> &TimelineState {
		&self.state
	}

	/// Generate timeline snapshot for UI rendering
	pub fn generate_timeline_snapshot(&self, current_time: Timestamp) -> Result<TimelineSnapshot> {
		let total_duration = current_time.saturating_sub(self.state.stream_start);

		// Create timeline segments by merging overlapping chapters by time
		let segments = self.create_timeline_segments(current_time, total_duration)?;

		let active_count = self.state.get_active_chapters_at(current_time).len();

		Ok(TimelineSnapshot {
			current_time,
			total_duration,
			segments,
			active_count,
			version: self.state.version,
		})
	}

	/// Create timeline segments for UI display
	fn create_timeline_segments(&self, current_time: Timestamp, total_duration: u64) -> Result<Vec<TimelineSegment>> {
		let mut segments = Vec::new();
		let mut timeline_points = BTreeMap::new();

		// Collect all time points where chapters start/end
		for chapter in self.state.chapters.values() {
			timeline_points.insert(chapter.time_range.start, ());
			if let Some(end) = chapter.time_range.end {
				timeline_points.insert(end, ());
			}
		}

		// Add stream start and current time
		timeline_points.insert(self.state.stream_start, ());
		timeline_points.insert(current_time, ());

		let time_points: Vec<Timestamp> = timeline_points.keys().cloned().collect();

		// Create segments between consecutive time points
		for i in 0..time_points.len().saturating_sub(1) {
			let start = time_points[i];
			let end = time_points[i + 1];

			if start >= end {
				continue;
			}

			// Find all chapters that overlap with this segment
			let overlapping_chapters: Vec<Chapter> = self
				.state
				.chapters
				.values()
				.filter(|chapter| {
					let range = TimeRange::new(start, Some(end));
					chapter.time_range.overlaps_with(&range)
				})
				.cloned()
				.collect();

			if !overlapping_chapters.is_empty() {
				// Use the title of the most recent chapter that starts in this segment
				let primary_chapter = overlapping_chapters
					.iter()
					.filter(|c| c.time_range.start <= start)
					.max_by_key(|c| c.time_range.start)
					.or_else(|| overlapping_chapters.first())
					.unwrap();

				let duration = end - start;
				let percentage = if total_duration > 0 { (duration as f64 / total_duration as f64) * 100.0 } else { 0.0 };

				let is_active = overlapping_chapters.iter().any(|c| c.is_active());

				segments.push(TimelineSegment {
					start_time: start,
					end_time: if end == current_time && is_active { None } else { Some(end) },
					duration,
					title: primary_chapter.get_display_title().to_string(),
					is_active,
					chapters: overlapping_chapters,
					percentage,
				});
			}
		}

		// If no segments created but we have active chapters, create a segment for the whole timeline
		if segments.is_empty() && !self.state.chapters.is_empty() {
			let all_chapters: Vec<Chapter> = self.state.chapters.values().cloned().collect();
			if let Some(primary_chapter) = all_chapters.first() {
				segments.push(TimelineSegment {
					start_time: self.state.stream_start,
					end_time: None,
					duration: total_duration,
					title: primary_chapter.get_display_title().to_string(),
					is_active: true,
					chapters: all_chapters,
					percentage: 100.0,
				});
			}
		}

		Ok(segments)
	}

	// Event handlers

	fn handle_start_chapter(&mut self, uid: Uid, context: Context, start_time: Timestamp, payload: Payload) -> Result<()> {
		let time_range = TimeRange::new(start_time, None);
		let chapter = Chapter::new(uid, context, time_range, payload);
		self.state.upsert_chapter(chapter);
		Ok(())
	}

	fn handle_end_chapter(&mut self, uid: Uid, end_time: Timestamp, final_payload: Option<Payload>) -> Result<()> {
		if let Some(chapter) = self.state.get_chapter_mut(&uid) {
			chapter.close_at(end_time)?;
			if let Some(payload) = final_payload {
				chapter.update_payload(payload);
			}
		}
		Ok(())
	}

	fn handle_update_payload(&mut self, uid: Uid, payload: Payload) -> Result<()> {
		if let Some(chapter) = self.state.get_chapter_mut(&uid) {
			chapter.update_payload(payload);
		}
		Ok(())
	}

	fn handle_update_context(&mut self, uid: Uid, context: Context) -> Result<()> {
		if let Some(chapter) = self.state.get_chapter_mut(&uid) {
			chapter.update_context(context);
		}
		Ok(())
	}

	fn handle_remove_chapter(&mut self, uid: Uid) -> Result<()> {
		if self.state.remove_chapter(&uid).is_some() {}
		Ok(())
	}

	fn handle_extend_chapter(&mut self, uid: Uid, extend_to: Timestamp) -> Result<()> {
		if let Some(chapter) = self.state.get_chapter_mut(&uid) {
			chapter.extend_to(extend_to)?;
		}
		Ok(())
	}

	fn handle_complete_chapter(&mut self, uid: Uid, completion_time: Timestamp, final_payload: Payload) -> Result<()> {
		if let Some(chapter) = self.state.get_chapter_mut(&uid) {
			chapter.close_at(completion_time)?;
			chapter.update_payload(final_payload);
		}
		Ok(())
	}

	fn handle_clear_all(&mut self) {
		self.state.clear_chapters();
	}
}

impl Default for LiveTimeline {
	fn default() -> Self {
		Self::new()
	}
}

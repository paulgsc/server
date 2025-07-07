use crate::error::*;
use crate::event::TimelineEvent;
use crate::state::{Chapter, TimelineState};
use crate::types::*;

pub struct LiveTimeline {
	state: TimelineState,
}

impl LiveTimeline {
	pub fn new() -> Self {
		Self { state: TimelineState::new() }
	}

	pub fn process_event(&mut self, event: TimelineEvent) -> Result<TimelineState> {
		match event {
			TimelineEvent::ChapterObservation {
				uid,
				context,
				time_range,
				payload,
			} => {
				self.handle_chapter_observation(uid, context, time_range, payload)?;
			}
			TimelineEvent::CloseChapter { uid, end_time, final_payload } => {
				self.handle_close_chapter(uid, end_time, final_payload)?;
			}
			TimelineEvent::RemoveChapter { uid } => {
				self.handle_remove_chapter(uid)?;
			}
			TimelineEvent::UpdatePayload { uid, payload } => {
				self.handle_update_payload(uid, payload)?;
			}
			TimelineEvent::UpdateContext { uid, context } => {
				self.handle_update_context(uid, context)?;
			}
			TimelineEvent::ExtendChapter { uid, extend_to } => {
				self.handle_extend_chapter(uid, extend_to)?;
			}
			TimelineEvent::ClearAll => {
				self.handle_clear_all()?;
			}
			TimelineEvent::AdvanceTime { current_time } => self.handle_advance_time(current_time),
		}

		Ok(self.state.clone())
	}

	pub fn current_state(&self) -> &TimelineState {
		&self.state
	}

	pub fn get_chapters_at(&self, timestamp: Timestamp) -> Vec<Chapter> {
		self.state.get_active_chapters_at(timestamp).into_iter().clone().collect()
	}

	pub fn generate_time_series(&self, start: Timestamp, end: Timestamp, interval: u64) -> Vec<crate::TimeSeriesPoint> {
		let mut points = Vec::new();
		let mut current = start;

		while current <= end {
			let chapters = self.get_chapters_at(current);
			let active_count = chapters.iter().filter(|c| c.is_active()).count();
			let total_duration = chapters.iter().map(|c| c.duration(current)).sum();

			points.push(crate::TimeSeriesPoint {
				timestamp: current,
				chapters,
				active_count,
				total_duration,
			});

			current += interval;
		}

		points
	}

	fn handle_chapter_observation(&mut self, uid: Uid, context: Context, time_range: TimeRange, payload: Payload) -> Result<()> {
		let chapter = Chapter::new(uid, context, time_range, payload);
		let _ = self.state.upsert_chapter(chapter);

		Ok(())
	}

	fn handle_close_chapter(&mut self, uid: Uid, end_time: Timestamp, final_payload: Option<Payload>) -> Result<()> {
		if let Some(chapter) = self.state.get_chapter_mut(&uid) {
			chapter.close_at(end_time)?;
			if let Some(payload) = final_payload {
				chapter.update_payload(payload);
			}
		}
		Ok(())
	}

	fn handle_remove_chapter(&mut self, uid: Uid) -> Result<()> {
		self.state.remove_chapter(&uid).is_some();
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

	fn handle_extend_chapter(&mut self, uid: Uid, extend_to: Timestamp) -> Result<()> {
		if let Some(chapter) = self.state.get_chapter_mut(&uid) {
			chapter.extend_to(extend_to)?;
		}
		Ok(())
	}

	fn handle_clear_all(&mut self) {
		self.state.clear_chapters();
	}

	fn handle_advance_time(&mut self, current_time: Timestamp) {
		self.state.update_current_time(current_time);
	}
}

impl Default for LiveTimeline {
	fn default() -> Self {
		Self::new()
	}
}

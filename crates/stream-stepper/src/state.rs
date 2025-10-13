use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents the current state of the timeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineState {
	/// All chapters indexed by UID
	pub chapters: HashMap<Uid, Chapter>,
	/// Current timeline time
	pub current_time: Timestamp,
	/// Stream start time
	pub stream_start: Timestamp,
	/// Last update timestamp
	pub last_updated: Timestamp,
	/// State version for change tracking
	pub version: u64,
}

impl TimelineState {
	/// Create a new empty state
	pub fn new() -> Self {
		let now = chrono::Utc::now().timestamp_millis() as u64;
		Self {
			chapters: HashMap::new(),
			current_time: now,
			stream_start: now,
			last_updated: now,
			version: 0,
		}
	}

	/// Get chapters that are active at a specific time
	pub fn get_active_chapters_at(&self, timestamp: Timestamp) -> Vec<&Chapter> {
		self.chapters.values().filter(|chapter| chapter.time_range.contains(timestamp)).collect()
	}

	/// Get all chapters ordered by start time
	pub fn get_chapters_ordered(&self) -> Vec<&Chapter> {
		let mut chapters: Vec<&Chapter> = self.chapters.values().collect();
		chapters.sort_by_key(|c| c.time_range.start);
		chapters
	}

	/// Get chapters that overlap with a time range
	pub fn get_chapters_in_range(&self, start: Timestamp, end: Timestamp) -> Vec<&Chapter> {
		let range = TimeRange::new(start, Some(end));
		self.chapters.values().filter(|chapter| chapter.time_range.overlaps_with(&range)).collect()
	}

	/// Get total stream duration
	pub fn get_total_duration(&self) -> u64 {
		self.current_time.saturating_sub(self.stream_start)
	}

	/// Check if a chapter exists
	pub fn has_chapter(&self, uid: &str) -> bool {
		self.chapters.contains_key(uid)
	}

	/// Get a chapter by UID
	pub fn get_chapter(&self, uid: &str) -> Option<&Chapter> {
		self.chapters.get(uid)
	}

	/// Get mutable reference to a chapter
	pub fn get_chapter_mut(&mut self, uid: &str) -> Option<&mut Chapter> {
		self.chapters.get_mut(uid)
	}

	/// Add or update a chapter
	pub fn upsert_chapter(&mut self, chapter: Chapter) -> bool {
		let uid = chapter.uid.clone();
		let is_new = !self.chapters.contains_key(&uid);
		self.chapters.insert(uid, chapter);
		self.increment_version();
		is_new
	}

	/// Remove a chapter
	pub fn remove_chapter(&mut self, uid: &str) -> Option<Chapter> {
		let removed = self.chapters.remove(uid);
		if removed.is_some() {
			self.increment_version();
		}
		removed
	}

	/// Clear all chapters
	pub fn clear_chapters(&mut self) {
		if !self.chapters.is_empty() {
			self.chapters.clear();
			self.increment_version();
		}
	}

	/// Update the current time
	pub fn update_current_time(&mut self, timestamp: Timestamp) {
		if timestamp > self.current_time {
			self.current_time = timestamp;
			self.last_updated = chrono::Utc::now().timestamp_millis() as u64;
			self.increment_version();
		}
	}

	/// Get currently active chapters
	pub fn get_active_chapters(&self) -> Vec<&Chapter> {
		self.get_active_chapters_at(self.current_time)
	}

	fn increment_version(&mut self) {
		self.version = self.version.wrapping_add(1);
	}
}

impl Default for TimelineState {
	fn default() -> Self {
		Self::new()
	}
}

/// A chapter in the timeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
	/// Unique identifier
	pub uid: Uid,
	/// Chapter context and metadata
	pub context: Context,
	/// Time range for this chapter
	pub time_range: TimeRange,
	/// Satellite data payload
	// TODO: Get rid of this
	pub payload: Payload,
	/// When this chapter was created
	pub created_at: Timestamp,
	/// When this chapter was last updated
	pub updated_at: Timestamp,
}

impl Chapter {
	/// Create a new chapter
	pub fn new(uid: Uid, context: Context, time_range: TimeRange, payload: Payload) -> Self {
		let now = chrono::Utc::now().timestamp_millis() as u64;
		Self {
			uid,
			context,
			time_range,
			payload,
			created_at: now,
			updated_at: now,
		}
	}

	/// Check if this chapter is currently active
	pub fn is_active(&self) -> bool {
		self.time_range.is_active()
	}

	/// Get the effective duration of this chapter
	pub fn duration(&self, current_time: Timestamp) -> u64 {
		self.time_range.duration(current_time)
	}

	/// Update the payload and timestamp
	pub fn update_payload(&mut self, payload: Payload) {
		self.payload = payload;
		self.updated_at = chrono::Utc::now().timestamp_millis() as u64;
	}

	/// Update the context and timestamp
	pub fn update_context(&mut self, context: Context) {
		self.context = context;
		self.updated_at = chrono::Utc::now().timestamp_millis() as u64;
	}

	/// Close this chapter at a specific time
	pub fn close_at(&mut self, end_time: Timestamp) -> crate::Result<()> {
		if end_time <= self.time_range.start {
			return Err(crate::ChapterError::InvalidTimestamp("End time must be after start time".to_string()));
		}
		self.time_range.end = Some(end_time);
		self.updated_at = chrono::Utc::now().timestamp_millis() as u64;
		Ok(())
	}

	/// Extend this chapter to a specific time
	pub fn extend_to(&mut self, extend_time: Timestamp) -> crate::Result<()> {
		let current_end = self.time_range.end.unwrap_or(extend_time);
		if extend_time < current_end {
			return Err(crate::ChapterError::InvalidTimestamp("Cannot extend chapter to earlier time".to_string()));
		}
		if self.time_range.end.is_some() {
			self.time_range.end = Some(extend_time);
		}
		self.updated_at = chrono::Utc::now().timestamp_millis() as u64;
		Ok(())
	}

	/// Get the title for UI display
	pub fn get_display_title(&self) -> &str {
		&self.context.title
	}
}

use crate::error::*;
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineState {
	pub chapters: HashMap<Uid, Chapter>,
	pub current_time: Timestamp,
	pub last_updated: Timestamp,
	pub version: u64,
}

impl TimelineState {
	pub fn new() -> Self {
		let now = chrono::Utc::now().timestamp_millis() as u64;
		Self {
			chapters: HashMap::new(),
			current_time: now,
			last_updated: now,
			version: 0,
		}
	}

	pub fn get_active_chapters_at(&self, timestamp: Timestamp) -> Vec<&Chapter> {
		self.chapters.values().filter(|chapter| chapter.time_range.contains(timestamp)).collect()
	}

	pub fn get_chapters_ordered(&self) -> Vec<&Chapter> {
		let mut chapters: Vec<&Chapter> = self.chapters.values().collect();
		chapters.sort_by_key(|c| c.time_range.start);
		chapters
	}

	pub fn get_chatpers_in_range(&self, start: Timestamp, end: Timestamp) -> Vec<&Chapter> {
		let range = TimeRange::new(start, Some(end));
		self.chapters.values().filter(|chapter| chapter.time_range.overlaps_with(&rang)).collect()
	}

	pub fn get_chapter(&self, uid: &str) -> Option<&Chapter> {
		self.chapters.get(uid)
	}

	pub fn get_chapter_mut(&mut self, uid: &str) -> Option<&mut Chapter> {
		self.chapters.get_mut(uid)
	}

	pub fn upsert_chapter(&mut self, chapter: Chapter) -> bool {
		let uid = chapter.uid.clone();
		let is_new = !self.chapters.contains_key(&uid);
		self.chapters.insert(uid, chapter);
		self.increment_version();
		is_new
	}

	pub fn remove_chapter(&mut self, uid: &str) -> Option<Chapter> {
		let removed = self.chapters.remove(uid);
		if removed.is_some() {
			self.increment_version();
		}
		removed
	}

	pub fn clear_chapters(&mut self) {
		if !self.chapters.is_empty() {
			self.chapters.clear();
			self.increment_version();
		}
	}

	pub fn update_current_time(&mut self, timestamp: Timestamp) {
		if timestamp > self.current_time {
			self.current_time = timestamp;
			self.last_updated = chrone::Utc::now().timestamp_millis() as u64;
			self.increment_version();
		}
	}

	pub fn get_active_chapters(&self) -> Vec<&Chapter> {
		self.get_active_chapters_at(self.current_time)
	}

	pub fn get_stats(&self) -> StateStats {
		let active_count = self.get_active_chapters().len();
		let total_chapters = self.chapters.len();
		let total_duration: u64 = self.chapters.values().map(|c| c.time_range.duration(self.current_time)).sum();

		StateStats {
			total_chapters,
			active_chapters: active_count,
			total_duration,
			current_time: self.current_time,
			version: self.version,
		}
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
	pub uid: Uid,
	pub context: Context,
	pub time_range: TimeRange,
	pub payload: Payload,
	pub created_at: Timestamp,
	pub updated_at: Timestamp,
}

impl Chapter {
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

	pub fn is_active(&self) -> bool {
		self.time_range.is_active()
	}

	pub fn duration(&self, current_time: Timestamp) -> u64 {
		self.time_range.duration(current_time)
	}

	pub fn update_payload(&mut self, payload: Payload) {
		self.payload = payload;
		self.updated_at = chrono::Utc::now().timestamp_millis() as u64;
	}

	pub fn update_context(&mut self, context: Context) {
		self.context = context;
		self.updated_at = chrono::Utc::now().timestamp_millis() as u64;
	}

	pub fn close_at(&mut self, end_time: Timestamp) -> crate::Result<()> {
		if end_time <= self.time_range.start {
			return Err(ChapterError::InvalidTimestamp("End time must be after start time".to_string()));
		}
		self.time_range.end = Some(end_time);
		self.updated_at = chrono::Utc::now().timestamp_millis() as u64;
		Ok(())
	}

	pub fn extend_to(&mut self, extend_time: Timestamp) -> crate::Result<()> {
		let current_end = self.time_range.end.unwrap_or(extend_time);
		if extend_time < current_end {
			return Err(ChapterError::InvalidTimestamp("Cannot extend chapter to earlier time".to_string()));
		}
		if self.time_range.end.is_some() {
			self.time_range.end = Some(extend_time);
		}
		self.updated_at = chrono::Utc::now().timestamp_millis() as u64;
		Ok(())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStats {
	pub total_chapters: usize,
	pub active_chapters: usize,
	pub total_duration: u64,
	pub current_time: Timestamp,
	pub version: u64,
}

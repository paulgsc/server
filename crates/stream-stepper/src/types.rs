use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Timestamp = u64;

pub type Uid = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Context {
	pub topic: String,
	pub tags: HashMap<String, String>,
	pub revision_tag: Option<String>,
}

impl Context {
	pub fn new(topic: String) -> Self {
		Self {
			topic,
			tags: HashMap::new(),
			revision_tag: None,
		}
	}

	pub fn with_tag(mut self, key: String, value: String) -> Self {
		self.tags.insert(key, value);
		self
	}

	pub fn with_revision_tag(mut self, revision_tag: String) -> Self {
		self.revision_tag = Some(revision_tag);
		self
	}

	pub fn is_same_chapter(&self, other: &self) -> bool {
		self.topic == other.topic && self.tags == other.tags
	}

	pub fn can_merge_with(&self, other: &Self) -> bool {
		self.is_same_chapter(other)
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
	pub data: serde_json::Value,
	pub metadata: Option<HashMap<String, String>>,
}

impl Payload {
	pub fn new<T: Serialize>(data: T) -> crate::Result<Self> {
		let data = serde_json::to_value(data)?;
		Ok(Self { data, metadata: None })
	}

	pub fn empty() -> Self {
		Self {
			data: serde_json::Value::Null,
			metadata: None,
		}
	}

	pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
		self.metadata = Some(metadata);
		self
	}

	pub fn get_data<T: for<'de> Deserialize<'de>>(&self) -> crate::Result<T> {
		Ok(serde_json::from_value(self.data.clone())?)
	}
}

impl Default for Payload {
	fn default() -> Self {
		Self::empty()
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeRange {
	pub start: Timestamp,
	pub end: Option<Timestamp>,
}

impl TimeRange {
	pub fn new(start: Timestamp, end: Option<Timestamp>) -> Self {
		Self { start, end }
	}

	pub fn is_active(&self) -> bool {
		self.end.is_none()
	}

	pub fn contains(&self, timestamp: Timestamp) -> bool {
		self.start <= timestamp && self.end.map_or(true, |end| timestamp < end)
	}

	pub fn overlaps_with(&self, other: &TimeRange) -> bool {
		let self_end = self.end.unwrap_or(Timestamp::Max);
		let other_end = other.end.unwrap_or(Timestamp::Max);
		self.start < other_end && other.start < self_end
	}

	pub fn duration(&self, current_time: Timestamp) -> u64 {
		let end = self.end.unwrap_or(current_time);
		end.saturating_sub(self.start)
	}

	pub fn effective_end(&self, current_time: Timestamp) -> Timestamp {
		self.end.unwrap_or(current_time)
	}
}

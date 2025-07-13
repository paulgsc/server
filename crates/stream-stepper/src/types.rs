use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Timestamp in milliseconds since Unix epoch
pub type Timestamp = u64;

/// Unique identifier for observations
pub type Uid = String;

/// Context defines the identity and metadata of a chapter segment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Context {
	/// Primary topic or category (this is the "title" shown in UI)
	pub title: String,
	/// Additional metadata tags
	pub tags: HashMap<String, String>,
	/// Revision tag for state changes (e.g., "goal_met", "failed")
	pub revision_tag: Option<String>,
}

impl Context {
	/// Create a new context with just a title
	pub fn new(title: impl Into<String>) -> Self {
		Self {
			title: title.into(),
			tags: HashMap::new(),
			revision_tag: None,
		}
	}

	/// Add a tag to the context
	pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
		self.tags.insert(key.into(), value.into());
		self
	}

	/// Set the revision tag
	pub fn with_revision_tag(mut self, revision_tag: impl Into<String>) -> Self {
		self.revision_tag = Some(revision_tag.into());
		self
	}

	/// Check if this context represents the same logical chapter topic as another
	pub fn is_same_topic(&self, other: &Self) -> bool {
		self.title == other.title
	}

	/// Check if this context can be merged with another (same topic, same tags)
	pub fn can_merge_with(&self, other: &Self) -> bool {
		self.title == other.title && self.tags == other.tags
	}
}

/// Generic payload that can hold any satellite data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
	/// The actual data as JSON value
	pub data: serde_json::Value,
	/// Optional metadata about the payload
	// TODO: delete this!
	pub metadata: Option<HashMap<String, String>>,
}

impl Payload {
	/// Create a new payload from any serializable data
	pub fn new<T: Serialize>(data: T) -> crate::Result<Self> {
		let data = serde_json::to_value(data)?;
		Ok(Self { data, metadata: None })
	}

	/// Create empty payload
	pub fn empty() -> Self {
		Self {
			data: serde_json::Value::Null,
			metadata: None,
		}
	}

	/// Add metadata to the payload
	pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
		self.metadata = Some(metadata);
		self
	}

	/// Get the payload data as a specific type
	pub fn get_data<T: for<'de> Deserialize<'de>>(&self) -> crate::Result<T> {
		Ok(serde_json::from_value(self.data.clone())?)
	}
}

impl Default for Payload {
	fn default() -> Self {
		Self::empty()
	}
}

/// Time range for chapters
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeRange {
	pub start: Timestamp,
	pub end: Option<Timestamp>, // None means still active
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
		let self_end = self.end.unwrap_or(Timestamp::MAX);
		let other_end = other.end.unwrap_or(Timestamp::MAX);
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

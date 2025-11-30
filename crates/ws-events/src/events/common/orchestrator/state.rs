use super::types::TimeMs;
use serde::{Deserialize, Serialize};

/// Stream status information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
	pub is_streaming: bool,
	pub stream_time: TimeMs,
	pub timecode: String,
}

impl StreamStatus {
	pub fn new() -> Self {
		Self {
			is_streaming: false,
			stream_time: 0,
			timecode: "00:00:00.000".to_string(),
		}
	}

	pub fn update(&mut self, is_streaming: bool, stream_time: TimeMs, timecode: String) {
		self.is_streaming = is_streaming;
		self.stream_time = stream_time;
		self.timecode = timecode;
	}
}

impl Default for StreamStatus {
	fn default() -> Self {
		Self::new()
	}
}

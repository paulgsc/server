use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
	/// Schema version for backwards/forwards compatibility
	pub version: EventVersion,
	/// Event timestamp (milliseconds since epoch)
	pub timestamp_ms: i64,
	/// Source service/instance identifier
	pub source: String,
	/// Event payload
	#[serde(flatten)]
	pub payload: EventPayload,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EventVersion {
	V1,
	V2,
}

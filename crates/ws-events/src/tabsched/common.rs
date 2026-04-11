#![cfg(feature = "tabsched")]

use prost::Message as ProstMessage;
use serde::{Deserialize, Serialize};

/// The message published onto the NATS JetStream subject
/// `pipeline.jobs`.  Small by design — no payload inline.
#[derive(Clone, Serialize, Deserialize, ProstMessage)]
pub struct JobEnvelope {
	/// Stable identifier; also the Redis key prefix.
	#[prost(string, tag = "1")]
	pub session_id: String,
	/// ISO-8601 origination time (from CaptureSession).
	#[prost(string, tag = "2")]
	pub captured_at: String,
	/// Monotonic retry counter set by the publisher, not the worker.
	/// Workers treat this as read-only.
	#[prost(uint32, tag = "3")]
	#[serde(default)]
	pub attempt: u32,
}

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Result of processing a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessResult {
	pub delivered: u64,
	pub failed: u64,
	pub duration: u64,
}

impl Default for ProcessResult {
	fn default() -> Self {
		Self {
			delivered: 0,
			failed: 0,
			duration: 0,
		}
	}
}

impl ProcessResult {
	pub fn success(delivered: u64, duration: u64) -> Self {
		Self { delivered, failed: 0, duration }
	}

	pub fn failure(failed: u64, duration: u64) -> Self {
		Self { delivered: 0, failed, duration }
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(u64);

impl MessageId {
	pub fn new() -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(1);
		Self(COUNTER.fetch_add(1, Ordering::Relaxed))
	}

	// Return the raw ID
	pub fn as_u64(&self) -> u64 {
		self.0
	}

	// Parse from string (useful for deserialization)
	pub fn from_str(s: &str) -> Option<Self> {
		s.strip_prefix("msg-").and_then(|n| n.parse::<u64>().ok()).map(Self)
	}
}

impl Default for MessageId {
	fn default() -> Self {
		Self::new()
	}
}

// Implement Display instead of custom to_string()
impl fmt::Display for MessageId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "msg-{}", self.0)
	}
}

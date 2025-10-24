use super::*;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::{SplitStream, StreamExt};
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_connection::ConnectionId;

/// Result of processing a message
#[derive(Debug, Clone)]
pub struct ProcessResult {
	pub delivered: usize,
	pub failed: usize,
	pub duration: Duration,
}

impl Default for ProcessResult {
	fn default() -> Self {
		Self {
			delivered: 0,
			failed: 0,
			duration: Duration::ZERO,
		}
	}
}

impl ProcessResult {
	pub fn success(delivered: usize, duration: Duration) -> Self {
		Self { delivered, failed: 0, duration }
	}

	pub fn failure(failed: usize, duration: Duration) -> Self {
		Self { delivered: 0, failed, duration }
	}
}

/// Client-originated messages (commands/requests from WebSocket clients)
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
	#[serde(rename = "pong")]
	Pong,

	#[serde(rename = "subscribe")]
	Subscribe { event_types: Vec<EventType> },

	#[serde(rename = "unsubscribe")]
	Unsubscribe { event_types: Vec<EventType> },

	#[serde(other)]
	Unknown { type_name: String },
}

// Message ID for tracking
#[derive(Debug, Clone, Copy)]
pub struct MessageId(u64);

impl MessageId {
	pub fn new() -> Self {
		use std::sync::atomic::{AtomicU64, Ordering};
		static COUNTER: AtomicU64 = AtomicU64::new(1);
		Self(COUNTER.fetch_add(1, Ordering::Relaxed))
	}

	pub fn to_string(&self) -> String {
		format!("msg-{}", self.0)
	}
}

impl Default for MessageId {
	fn default() -> Self {
		Self::new()
	}
}

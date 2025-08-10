use super::{ConnectionId, ProcessResult};
use crate::{utils::generate_uuid, UtteranceMetadata};
use obs_websocket::ObsEvent;
use serde::{Deserialize, Serialize};
use std::{
	fmt,
	sync::atomic::{AtomicU64, Ordering},
};
use tokio::time::{Duration, Instant};

// Message correlation ID for tracing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageId([u8; 32]);

impl MessageId {
	pub fn new() -> Self {
		Self(generate_uuid())
	}

	pub fn as_string(&self) -> String {
		hex::encode(&self.0)
	}
}

impl fmt::Display for MessageId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.as_string())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
	ObsStatus,
	ClientCount,
	Ping,
	Pong,
	Error,
	TabMetaData,
	Utterance,
}

#[derive(Clone, Serialize, Debug, Deserialize)]
pub struct NowPlaying {
	title: String,
	channel: String,
	video_id: String,
	current_time: u32,
	duration: u32,
	thumbnail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Event {
	ObsStatus { status: ObsEvent },
	TabMetaData { data: NowPlaying },
	ClientCount { count: usize },
	Ping,
	Pong,
	Error { message: String },
	Subscribe { event_types: Vec<EventType> },
	Unsubscribe { event_types: Vec<EventType> },
	Utterance { text: String, metadata: UtteranceMetadata },
}

impl Event {
	pub fn get_type(&self) -> EventType {
		match self {
			Self::Ping => EventType::Ping,
			Self::Pong => EventType::Pong,
			Self::Error { .. } => EventType::Error,
			Self::Subscribe { .. } => EventType::Ping, // These are control messages
			Self::Unsubscribe { .. } => EventType::Ping,
			Self::ClientCount { .. } => EventType::ClientCount,
			Self::ObsStatus { .. } => EventType::ObsStatus,
			Self::TabMetaData { .. } => EventType::TabMetaData,
			Self::Utterance { .. } => EventType::Utterance,
		}
	}
}

impl From<NowPlaying> for Event {
	fn from(data: NowPlaying) -> Self {
		Event::TabMetaData { data }
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UtterancePrompt {
	pub text: String,
	pub metadata: UtteranceMetadata,
}

impl From<UtterancePrompt> for Event {
	fn from(UtterancePrompt { text, metadata }: UtterancePrompt) -> Self {
		Event::Utterance { text, metadata }
	}
}

#[derive(Debug, Clone)]
pub enum ConnectionState {
	Active { last_ping: Instant },
	Stale { last_ping: Instant, reason: String },
	Disconnected { reason: String, disconnected_at: Instant },
}

impl fmt::Display for ConnectionState {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Active { last_ping } => {
				write!(f, "Active (last_ping: {:?})", last_ping)
			}
			Self::Stale { last_ping, reason } => {
				write!(f, "Stale (last_ping: {:?}, reason: {})", last_ping, reason)
			}
			Self::Disconnected { reason, disconnected_at } => {
				write!(f, "Disconnected (reason: {}, at: {:?})", reason, disconnected_at)
			}
		}
	}
}

// System event for observability and debugging
#[derive(Debug, Clone)]
pub enum SystemEvent {
	ConnectionStateChanged {
		connection_id: ConnectionId,
		from: ConnectionState,
		to: ConnectionState,
	},
	MessageProcessed {
		message_id: MessageId,
		connection_id: ConnectionId,
		duration: Duration,
		result: ProcessResult,
	},
	BroadcastFailed {
		event_type: EventType,
		error: String,
		affected_connections: usize,
	},
	ConnectionCleanup {
		connection_id: ConnectionId,
		reason: String,
		resources_freed: bool,
	},
}

// Connection metrics for monitoring
#[derive(Debug, Default)]
pub struct ConnectionMetrics {
	pub total_created: AtomicU64,
	pub total_removed: AtomicU64,
	pub current_active: AtomicU64,
	pub current_stale: AtomicU64,
	pub messages_processed: AtomicU64,
	pub messages_failed: AtomicU64,
	pub broadcast_succeeded: AtomicU64,
	pub broadcast_failed: AtomicU64,
}

impl ConnectionMetrics {
	pub fn connection_created(&self) {
		self.total_created.fetch_add(1, Ordering::Relaxed);
		self.current_active.fetch_add(1, Ordering::Relaxed);
	}

	pub fn connection_removed(&self, was_active: bool) {
		self.total_removed.fetch_add(1, Ordering::Relaxed);
		if was_active {
			self.current_active.fetch_sub(1, Ordering::Relaxed);
		} else {
			self.current_stale.fetch_sub(1, Ordering::Relaxed);
		}
	}

	pub fn connection_marked_stale(&self) {
		self.current_active.fetch_sub(1, Ordering::Relaxed);
		self.current_stale.fetch_add(1, Ordering::Relaxed);
	}

	pub fn message_processed(&self, success: bool) {
		if success {
			self.messages_processed.fetch_add(1, Ordering::Relaxed);
		} else {
			self.messages_failed.fetch_add(1, Ordering::Relaxed);
		}
	}

	pub fn broadcast_attempt(&self, success: bool) {
		if success {
			self.broadcast_succeeded.fetch_add(1, Ordering::Relaxed);
		} else {
			self.broadcast_failed.fetch_add(1, Ordering::Relaxed);
		}
	}

	pub fn get_snapshot(&self) -> ConnectionMetricsSnapshot {
		ConnectionMetricsSnapshot {
			total_created: self.total_created.load(Ordering::Relaxed),
			total_removed: self.total_removed.load(Ordering::Relaxed),
			current_active: self.current_active.load(Ordering::Relaxed),
			current_stale: self.current_stale.load(Ordering::Relaxed),
			messages_processed: self.messages_processed.load(Ordering::Relaxed),
			messages_failed: self.messages_failed.load(Ordering::Relaxed),
			broadcast_succeeded: self.broadcast_succeeded.load(Ordering::Relaxed),
			broadcast_failed: self.broadcast_failed.load(Ordering::Relaxed),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionMetricsSnapshot {
	pub total_created: u64,
	pub total_removed: u64,
	pub current_active: u64,
	pub current_stale: u64,
	pub messages_processed: u64,
	pub messages_failed: u64,
	pub broadcast_succeeded: u64,
	pub broadcast_failed: u64,
}

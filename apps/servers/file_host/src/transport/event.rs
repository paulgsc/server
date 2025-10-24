use super::ProcessResult;
use crate::{utils::generate_uuid, UtteranceMetadata};
use obs_websocket::{ObsCommand, ObsEvent};
use serde::{Deserialize, Serialize};
use std::{
	collections::HashSet,
	fmt,
	sync::atomic::{AtomicU64, Ordering},
};
use tokio::time::Duration;
use ws_connection::{ConnectionId, ConnectionState};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
	ObsStatus,
	ObsCommand,
	ClientCount,
	Ping,
	Pong,
	Error,
	TabMetaData,
	Utterance,
}

impl Default for EventType {
	fn default() -> Self {
		EventType::Pong
	}
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
	ObsStatus {
		status: ObsEvent,
	},
	ObsCmd {
		cmd: ObsCommand,
	},
	TabMetaData {
		data: NowPlaying,
	},
	ClientCount {
		count: usize,
	},
	Ping,
	Pong,
	Error {
		message: String,
	},
	Subscribe {
		event_types: Vec<EventType>,
	},
	Unsubscribe {
		event_types: Vec<EventType>,
	},
	Utterance {
		text: String,
		metadata: UtteranceMetadata,
	},

	// System/observability events (not sent to clients)
	#[serde(skip)]
	ConnectionStateChanged {
		connection_id: ConnectionId,
		from: ConnectionState,
		to: ConnectionState,
	},
	#[serde(skip)]
	MessageProcessed {
		message_id: MessageId,
		connection_id: ConnectionId,
		duration: Duration,
		result: ProcessResult,
	},
	#[serde(skip)]
	BroadcastFailed {
		event_type: EventType,
		error: String,
		affected_connections: usize,
	},
	#[serde(skip)]
	ConnectionCleanup {
		connection_id: ConnectionId,
		reason: String,
		resources_freed: bool,
	},
}

impl Event {
	/// Check if this event should be sent to clients
	pub fn is_client_event(&self) -> bool {
		!matches!(
			self,
			Self::ConnectionStateChanged { .. } | Self::MessageProcessed { .. } | Self::BroadcastFailed { .. } | Self::ConnectionCleanup { .. }
		)
	}

	/// Check if this is a system/observability event
	pub fn is_system_event(&self) -> bool {
		!self.is_client_event()
	}

	pub fn get_type(&self) -> Option<EventType> {
		match self {
			Self::Ping => Some(EventType::Ping),
			Self::Pong => Some(EventType::Pong),
			Self::Error { .. } => Some(EventType::Error),
			Self::Subscribe { .. } => Some(EventType::Ping), // These are control messages
			Self::Unsubscribe { .. } => Some(EventType::Ping),
			Self::ClientCount { .. } => Some(EventType::ClientCount),
			Self::ObsStatus { .. } => Some(EventType::ObsStatus),
			Self::ObsCmd { .. } => Some(EventType::ObsCommand),
			Self::TabMetaData { .. } => Some(EventType::TabMetaData),
			Self::Utterance { .. } => Some(EventType::Utterance),
			// System events don't have EventTypes
			_ => None,
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

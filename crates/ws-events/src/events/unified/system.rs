#![cfg(feature = "events")]

use crate::events::{EventType, ProcessResult};
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct ClientCountMessage {
	#[prost(uint64, tag = "1")]
	pub count: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ErrorMessage {
	#[prost(string, tag = "1")]
	pub message: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ConnectionStateChangedMessage {
	#[prost(string, tag = "1")]
	pub connection_id: String,
	#[prost(string, tag = "2")]
	pub from_state: String,
	#[prost(string, tag = "3")]
	pub to_state: String,
	#[prost(int64, tag = "4")]
	pub timestamp: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct MessageProcessedMessage {
	#[prost(string, tag = "1")]
	pub message_id: String,
	#[prost(string, tag = "2")]
	pub connection_id: String,
	#[prost(uint64, tag = "3")]
	pub duration_micros: u64,
	#[prost(uint64, tag = "4")]
	pub delivered: u64,
	#[prost(uint64, tag = "5")]
	pub failed: u64,
	#[prost(int64, tag = "6")]
	pub timestamp: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct BroadcastFailedMessage {
	#[prost(string, tag = "1")]
	pub event_type: String,
	#[prost(string, tag = "2")]
	pub error: String,
	#[prost(uint64, tag = "3")]
	pub affected_connections: u64,
	#[prost(int64, tag = "4")]
	pub timestamp: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ConnectionCleanupMessage {
	#[prost(string, tag = "1")]
	pub connection_id: String,
	#[prost(string, tag = "2")]
	pub reason: String,
	#[prost(bool, tag = "3")]
	pub resources_freed: bool,
	#[prost(int64, tag = "4")]
	pub timestamp: i64,
}

impl ConnectionStateChangedMessage {
	pub fn new(connection_id: String, from: String, to: String) -> Self {
		Self {
			connection_id,
			from_state: from,
			to_state: to,
			timestamp: chrono::Utc::now().timestamp(),
		}
	}
}

impl MessageProcessedMessage {
	pub fn new(message_id: String, connection_id: String, result: ProcessResult) -> Self {
		Self {
			message_id,
			connection_id,
			duration_micros: result.duration,
			delivered: result.delivered,
			failed: result.failed,
			timestamp: chrono::Utc::now().timestamp(),
		}
	}
}

impl BroadcastFailedMessage {
	pub fn new(event_type: EventType, error: String, affected_connections: u64) -> Self {
		Self {
			event_type: format!("{:?}", event_type),
			error,
			affected_connections: affected_connections as u64,
			timestamp: chrono::Utc::now().timestamp(),
		}
	}
}

impl ConnectionCleanupMessage {
	pub fn new(connection_id: String, reason: String, resources_freed: bool) -> Self {
		Self {
			connection_id,
			reason,
			resources_freed,
			timestamp: chrono::Utc::now().timestamp(),
		}
	}
}

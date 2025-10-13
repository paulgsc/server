#![cfg(feature = "inmem")]

/// Transport-agnostic error type
#[derive(Debug, thiserror::Error, Clone)]
pub enum TransportError {
	/// The channel or connection is closed
	#[error("Transport channel closed")]
	Closed,

	/// The receiver lagged and messages were dropped
	#[error("Transport overflowed, {0} messages dropped")]
	Overflowed(u64),

	/// Failed to send a message
	#[error("Failed to send: {0}")]
	SendFailed(String),

	/// Failed to broadcast a message
	#[error("Broadcast failed: {0}")]
	BroadcastFailed(String),

	/// Connection not found
	#[error("Connection {0} not found")]
	ConnectionNotFound(String),

	/// Generic transport error
	#[error("Transport error: {0}")]
	Other(String),
}

/// Result type for transport operations
pub type Result<T> = std::result::Result<T, TransportError>;

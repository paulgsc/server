use thiserror::Error;

/// Result type for transport operations
pub type Result<T> = std::result::Result<T, TransportError>;

/// Unified error type for all transport implementations.
///
/// This error type covers common failure modes across different transports
/// (in-memory, NATS, Redis, etc.) as well as transport-specific errors.
///
/// # Example
///
/// ```rust,no_run
/// use some_transport::error::{Result, TransportError};
///
/// async fn send_message() -> Result<()> {
///     // ... transport operations
///     Err(TransportError::Closed)
/// }
/// ```
#[derive(Error, Debug, Clone)]
pub enum TransportError {
	/// The channel or connection is closed
	#[error("Transport channel closed")]
	Closed,

	/// The receiver lagged and messages were dropped
	#[error("Channel overflowed, {0} messages dropped")]
	Overflowed(u64),

	/// Connection not found for the given key
	#[error("Connection not found: {0}")]
	ConnectionNotFound(String),

	/// Failed to send a message to a specific connection
	#[error("Failed to send message: {0}")]
	SendFailed(String),

	/// Failed to broadcast a message to all connections
	#[error("Failed to broadcast message: {0}")]
	BroadcastFailed(String),

	/// Serialization error (used by transports that serialize messages)
	#[cfg(feature = "nats")]
	#[error("Serialization error: {0}")]
	SerializationError(String),

	/// Deserialization error (used by transports that deserialize messages)
	#[cfg(feature = "nats")]
	#[error("Deserialization error: {0}")]
	DeserializationError(String),

	/// NATS-specific error
	#[cfg(feature = "nats")]
	#[error("NATS error: {0}")]
	NatsError(String),

	/// Generic transport error
	#[error("Transport error: {0}")]
	Other(String),
}

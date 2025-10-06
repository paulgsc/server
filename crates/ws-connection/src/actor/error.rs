use thiserror::Error;

/// Result type alias for connection operations
pub type Result<T> = std::result::Result<T, ConnectionError>;

/// Errors that can occur during connection actor operations
#[derive(Debug, Error)]
pub enum ConnectionError {
	/// The connection actor is no longer available
	#[error("Connection actor unavailable")]
	ActorUnavailable,

	/// Failed to receive state from the actor
	#[error("Failed to get state from connection actor")]
	StateRetrievalFailed,

	/// Multiple Arc references detected when unique ownership is required
	#[error("Multiple Arc references to connection detected during {operation}")]
	MultipleArcReferences { operation: String },
}

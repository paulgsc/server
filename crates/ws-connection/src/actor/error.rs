use thiserror::Error;
use tokio::sync::oneshot;

/// Result type alias for connection operations
pub type Result<T> = std::result::Result<T, ConnectionError>;

/// Errors that can occur during connection actor operations
#[derive(Debug, Error)]
pub enum ConnectionError {
	/// The connection actor is no longer available
	#[error("Connection actor unavailable: {0}")]
	ActorUnavailable(#[from] Box<dyn std::error::Error + Send + Sync>),

	/// Failed to receive state from the actor
	#[error("Failed to get state from connection actor: {0}")]
	StateRetrievalFailed(#[from] oneshot::error::RecvError),

	/// Multiple Arc references detected
	#[error("Multiple Arc references to connection detected during {operation}")]
	MultipleArcReferences { operation: String },
}

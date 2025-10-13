use crate::StateError;

/// Errors that can occur during polling operations
#[derive(Debug, thiserror::Error)]
pub enum PollingError {
	#[error("WebSocket send error: {0}")]
	WebSocketSend(#[from] tokio_tungstenite::tungstenite::Error),

	#[error("State validation error: {0}")]
	StateValidation(#[from] StateError),

	#[error("Command channel closed unexpectedly")]
	ChannelClosed,

	// #[error("Polling frequency timer error: {0}")]
	// TimerError(String),
	#[error("JSON serialization error: {0}")]
	JsonSerialization(#[from] serde_json::Error),

	#[error("Polling loop terminated due to critical error: {reason}")]
	CriticalLoopTermination { reason: String },
}

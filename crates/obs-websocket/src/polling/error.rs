use super::*;

/// Errors that can occur during polling operations
#[derive(Debug, thiserror::Error)]
pub enum PollingError {
	#[error("WebSocket send error: {0}")]
	WebSocketSend(#[from] tokio_tungstenite::tungstenite::Error),

	#[error("State validation error: {0}")]
	StateValidation(#[from] StateError),

	#[error("Failed to send {failed_count} out of {total_count} requests")]
	BatchSendFailure {
		failed_count: usize,
		total_count: usize,
		#[source]
		first_error: tokio_tungstenite::tungstenite::Error,
	},

	#[error("Failed to flush WebSocket sink after sending {request_count} requests")]
	FlushFailure {
		request_count: usize,
		#[source]
		error: tokio_tungstenite::tungstenite::Error,
	},

	#[error("Command channel closed unexpectedly")]
	ChannelClosed,

	// #[error("Polling frequency timer error: {0}")]
	// TimerError(String),
	#[error("JSON serialization error: {0}")]
	JsonSerialization(#[from] serde_json::Error),

	#[error("Polling loop terminated due to critical error: {reason}")]
	CriticalLoopTermination { reason: String },
}

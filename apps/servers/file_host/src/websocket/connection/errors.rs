use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConnectionError {
	#[error("Failed to get connection state: {0}")]
	StateRetrievalFailed(String),

	#[error("Failed to gracefully shutdown connection actor: {0}")]
	ShutdownFailed(String),

	#[error("Failed to close transport channel: {0}")]
	TransportCloseFailed(String),

	#[error("Connection cleanup failed: {0}")]
	CleanupFailed(String),

	#[error("Failed to subscribe to events: {0}")]
	SubscriptionFailed(ws_connection::actor::ConnectionError),

	#[error("Failed to send initial handshake: {0}")]
	HandshakeFailed(String),

	#[error("Failed to serialize message: {0}")]
	SerializationFailed(#[from] serde_json::Error),

	#[error("WebSocket send error: {0}")]
	WebSocketSendError(String),
}

impl From<axum::Error> for ConnectionError {
	fn from(err: axum::Error) -> Self {
		ConnectionError::WebSocketSendError(err.to_string())
	}
}

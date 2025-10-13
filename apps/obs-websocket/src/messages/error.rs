/// Custom error type for OBS WebSocket operations
#[derive(Debug, thiserror::Error)]
pub enum ObsMessagesError {
	#[error("WebSocket send error: {0}")]
	WebSocketSend(#[from] tokio_tungstenite::tungstenite::Error),

	#[error("JSON parsing error: {0}")]
	JsonParse(#[from] serde_json::Error),

	#[error("Missing required field: {field} in message type: {message_type}")]
	MissingField { field: String, message_type: String },

	#[error("Invalid field type: expected {expected} for field {field}")]
	InvalidFieldType { field: String, expected: String },

	#[error("Unknown operation code: {op_code}")]
	UnknownOpCode { op_code: u64 },

	#[error("OBS request failed - Type: {request_type}, Code: {code}, Comment: {comment}")]
	ObsRequestFailed { request_type: String, code: u64, comment: String },

	#[error("Initialization timeout")]
	InitializationTimeout,
}

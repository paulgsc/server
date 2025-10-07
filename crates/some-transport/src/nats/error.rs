#[derive(Debug, thiserror::Error)]
pub enum NatsError {
	#[error("NATS connection error: {0}")]
	ConnectionError(#[from] async_nats::ConnectError),

	#[error("NATS publish error: {0}")]
	PublishError(#[from] async_nats::PublishError),

	#[error("Serialization error: {0}")]
	SerializationError(#[from] serde_json::Error),

	#[error("Channel send error")]
	ChannelError,

	#[error("Invalid subject: {0}")]
	InvalidSubject(String),
}

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
	#[error("OBS WebSocket error: {0}")]
	ObsWebSocket(#[from] obs_websocket::ObsWebsocketError),

	#[error("NATS transport error: {0}")]
	Transport(#[from] some_transport::TransportError),

	#[error("Serialization error: {0}")]
	Serialization(String),

	#[error("Deserialization error: {0}")]
	Deserialization(String),

	#[error("Service not connected")]
	NotConnected,

	#[error("Service shutdown")]
	Shutdown,

	#[error("Configuration error: {0}")]
	Config(String),

	#[error("Timeout: {0}")]
	Timeout(String),

	#[error("{0}")]
	Other(String),

	#[error("JSON parsing error: {0}")]
	JsonParse(#[from] serde_json::Error),
}

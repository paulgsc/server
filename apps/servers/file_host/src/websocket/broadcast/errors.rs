use some_transport::TransportError;
use thiserror::Error;
use ws_events::events::EventType;

#[derive(Error, Debug)]
pub enum BroadcastError {
	#[error("Event has no type")]
	NoEventType,

	#[error("Transport error during broadcast: {0}")]
	Transport(#[from] TransportError),

	#[error("Other: {0}")]
	Other(String),

	#[error("Event Lagged by {1} messages on subject {0:?}")]
	Lagged(EventType, u64),

	#[error("Broadcast channel closed for subject {0:?}")]
	Closed(EventType),

	#[error("No receivers for broadcast")]
	NoReceivers,
}

impl From<String> for BroadcastError {
	fn from(s: String) -> Self {
		BroadcastError::Other(s)
	}
}

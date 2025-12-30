use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum ConnectionError {
	#[error("connection not active (current state: {state})")]
	NotActive { state: String },

	#[error("invalid state transition from {from} to {to}: {reason}")]
	InvalidTransition { from: String, to: String, reason: String },

	#[error("channel send failed: {0}")]
	ChannelSend(String),

	#[error("connection not found")]
	NotFound,

	#[error("client limit exceeded: {limit}")]
	ClientLimitExceeded { limit: usize },

	#[error("invalid client id format")]
	InvalidClientId,

	#[error("subscription error: {0}")]
	Subscription(String),
}

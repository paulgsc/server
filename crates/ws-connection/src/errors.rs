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

#[derive(Error, Debug)]
pub enum NotifyError {
	#[error("message was dropped due to channel overflow")]
	Dropped,

	#[error("no active receivers")]
	NoReceivers,

	#[error("channel error: {0}")]
	Channel(String),
}

#[derive(Debug, Clone)]
pub enum SendOutcome {
	Sent,
	DroppedOldest,
	NoReceivers,
	Error(String),
}

impl From<NotifyError> for SendOutcome {
	fn from(err: NotifyError) -> Self {
		match err {
			NotifyError::Dropped => SendOutcome::DroppedOldest,
			NotifyError::NoReceivers => SendOutcome::NoReceivers,
			NotifyError::Channel(msg) => SendOutcome::Error(msg),
		}
	}
}

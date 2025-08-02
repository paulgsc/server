#[derive(Debug, thiserror::Error)]
pub enum SendError {
	#[error("Connection closed")]
	ConnectionClosed,

	#[error("Queue full")]
	QueueFull,

	#[error("Flow control blocked")]
	FlowControlBlocked,

	#[error("Message too large")]
	MessageTooLarge,

	#[error("Invalid message type")]
	InvalidMessageType,

	#[error("Timeout")]
	Timeout,
}

#[derive(Debug, thiserror::Error)]
pub enum KeepaliveError {
	#[error("Invalid pong data")]
	InvalidPongData,

	#[error("Unexpected pong")]
	UnexpectedPong,

	#[error("Ping timeout")]
	PingTimeout,
}

#[derive(Debug, thiserror::Error)]
pub enum BufferError {
	#[error("Send queue full")]
	SendQueueFull,

	#[error("Receive queue full")]
	ReceiveQueueFull,

	#[error("Buffer overflow")]
	BufferOverflow,
}

#[derive(Debug, thiserror::Error)]
pub enum FlowControlError {
	#[error("Insufficient credits: available={available}, required={required}")]
	InsufficientCredits { available: i32, required: i32 },

	#[error("Flow control disabled")]
	Disabled,
}

use super::error::{ConfigError, TransportError};
use crate::connection::endpoint::{ConnectionInfo, ConnectionStatistics, Endpoint};
use crate::message::{
	content::{OutgoingMessage, TransportMessage},
	error::SendError,
	metadata::MessageId,
};
use crate::protocol::types::CloseCode;
use bytes::Bytes;
use tokio::sync::oneshot;
use tokio::time::{Duration, Instant};

#[derive(Debug)]
pub struct TransportConfig {
	pub keepalive: KeepaliveConfig,
	pub flow_control: FlowControlConfig,
	pub buffer: BufferConfig,
}

#[derive(Debug, Clone)]
pub struct KeepaliveConfig {
	pub enabled: bool,
	pub interval: Duration,
	pub timeout: Duration,
	pub idle_timeout: Duration,
	pub max_failures: u32,
}

#[derive(Debug)]
pub struct FlowControlConfig {
	pub send_buffer_size: usize,
	pub receive_buffer_size: usize,
	pub backpressure_threshold: i32,
}

impl FlowControlConfig {
	pub fn initial_send_credits(&self) -> i32 {
		1000
	}
	pub fn initial_receive_credits(&self) -> i32 {
		1000
	}
	pub fn bytes_per_credit(&self) -> usize {
		1024
	}
}

#[derive(Debug)]
pub struct BufferConfig {
	pub send_queue_size: usize,
	pub receive_queue_size: usize,
	pub frame_buffer_size: usize,
	pub message_buffer_size: usize,
	pub max_send_queue_size: usize,
	pub max_receive_queue_size: usize,
	pub queue_policy: QueuePolicy,
}

#[derive(Debug, Clone, Copy)]
pub enum QueuePolicy {
	DropOldest,
	DropLowestPriority,
	RejectNew,
}

#[derive(Debug)]
pub enum TransportCommand {
	Connect {
		endpoint: Endpoint,
		config: TransportConfig,
		respond_to: oneshot::Sender<Result<ConnectionInfo, TransportError>>,
	},
	Disconnect {
		close_code: CloseCode,
		reason: Option<String>,
		respond_to: oneshot::Sender<Result<(), TransportError>>,
	},
	SendMessage {
		message: OutgoingMessage,
		respond_to: oneshot::Sender<Result<MessageId, SendError>>,
	},
	SendPing {
		data: Option<Bytes>,
		respond_to: oneshot::Sender<Result<(), TransportError>>,
	},
	UpdateConfig {
		config: TransportConfig,
		respond_to: oneshot::Sender<Result<(), ConfigError>>,
	},
	GetStatistics {
		respond_to: oneshot::Sender<ConnectionStatistics>,
	},
	SetFlowControl {
		enabled: bool,
		respond_to: oneshot::Sender<Result<(), TransportError>>,
	},
}

#[derive(Debug)]
pub enum TransportEvent {
	StateChanged {
		from: TransportStateInfo,
		to: TransportStateInfo,
		timestamp: Instant,
	},
	MessageReceived {
		message: TransportMessage,
		timestamp: Instant,
	},
	MessageSent {
		message_id: MessageId,
		size: usize,
		timestamp: Instant,
	},
	PingReceived {
		data: Option<Bytes>,
		timestamp: Instant,
	},
	PongReceived {
		data: Option<Bytes>,
		rtt: Duration,
		timestamp: Instant,
	},
	ConnectionClosed {
		code: CloseCode,
		reason: Option<String>,
		timestamp: Instant,
	},
	Error {
		error: TransportError,
		recoverable: bool,
		timestamp: Instant,
	},
	FlowControlUpdate {
		send_buffer_level: f32,
		receive_buffer_level: f32,
		timestamp: Instant,
	},
}

#[derive(Debug)]
pub enum TransportStateInfo {
	Idle,
	Connecting { endpoint: Endpoint },
	Connected { endpoint: Endpoint, connection_info: ConnectionInfo },
	Closing { code: CloseCode, reason: Option<String> },
	Failed { error: TransportError },
}

use crate::connection::endpoint::Endpoint;
use crate::protocol::types::{CloseCode, CloseInitiator};
use bytes::Bytes;
use tokio::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
	#[error("Connection error: {source}")]
	Connection {
		source: ConnectionError,
		endpoint: Endpoint,
		retry_after: Option<Duration>,
	},

	#[error("Protocol violation: {details}")]
	ProtocolViolation { details: String, raw_data: Option<Bytes> },

	#[error("Connection closed: code={code:?}, reason={reason:?}")]
	ConnectionClosed {
		code: CloseCode,
		reason: Option<String>,
		initiated_by: CloseInitiator,
	},

	#[error("Invalid state: current={current_state}, expected one of {expected_states:?}")]
	InvalidState { current_state: String, expected_states: Vec<String> },

	#[error("Message too large: size={size}, max={max_size}")]
	MessageTooLarge { size: usize, max_size: usize },

	#[error("Timeout: operation={operation}, duration={duration:?}")]
	Timeout { operation: String, duration: Duration },
}

impl TransportError {
	pub fn is_retryable(&self) -> bool {
		match self {
			Self::Connection { .. } => true,
			Self::Timeout { .. } => true,
			Self::ConnectionClosed { code, .. } => matches!(code, CloseCode::GoingAway | CloseCode::ServiceRestart | CloseCode::TryAgainLater),
			_ => false,
		}
	}

	pub fn severity(&self) -> ErrorSeverity {
		match self {
			Self::Connection { .. } => ErrorSeverity::High,
			Self::ProtocolViolation { .. } => ErrorSeverity::Medium,
			Self::ConnectionClosed { .. } => ErrorSeverity::Medium,
			Self::InvalidState { .. } => ErrorSeverity::High,
			Self::MessageTooLarge { .. } => ErrorSeverity::Low,
			Self::Timeout { .. } => ErrorSeverity::Medium,
		}
	}

	pub fn type_name(&self) -> &'static str {
		match self {
			Self::Connection { .. } => "Connection",
			Self::ProtocolViolation { .. } => "ProtocolViolation",
			Self::ConnectionClosed { .. } => "ConnectionClosed",
			Self::InvalidState { .. } => "InvalidState",
			Self::MessageTooLarge { .. } => "MessageTooLarge",
			Self::Timeout { .. } => "Timeout",
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
	#[error("TCP connection failed: {0}")]
	TcpConnection(Box<dyn std::error::Error + Send + Sync>),

	#[error("TLS handshake failed: {0}")]
	TlsHandshake(Box<dyn std::error::Error + Send + Sync>),

	#[error("WebSocket handshake failed: {0}")]
	WebSocketHandshake(Box<dyn std::error::Error + Send + Sync>),

	#[error("DNS resolution failed: {0}")]
	DnsResolution(Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
	#[error("Invalid configuration: {field}")]
	InvalidField { field: String },

	#[error("Configuration conflict: {details}")]
	Conflict { details: String },
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorSeverity {
	Low,
	Medium,
	High,
	Critical,
}

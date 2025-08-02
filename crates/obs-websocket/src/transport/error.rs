#[derive(Debug, thiserror::Error)]
pub enum TransportError {
	#[error("Connection failed: {source}")]
	Connection {
		#[from]
		source: ConnectionError,
		endpoint: Endpoint,
		retry_after: Option<Duration>,
	},

	#[error("Handshake failed: {reason}")]
	Handshake {
		reason: String,
		status_code: Option<u16>,
		headers: HashMap<String, String>,
	},

	#[error("Protocol error: {message}")]
	Protocol {
		message: String,
		frame_type: Option<FrameType>,
		close_code: Option<CloseCode>,
	},

	#[error("TLS error: {source}")]
	Tls {
		#[from]
		source: TlsError,
		certificate_issue: Option<CertificateError>,
	},

	#[error("Timeout: {operation}")]
	Timeout { operation: String, duration: Duration, partial_completion: bool },

	#[error("Buffer overflow: {buffer_type}")]
	BufferOverflow {
		buffer_type: String,
		current_size: usize,
		max_size: usize,
		dropped_messages: u64,
	},

	#[error("Flow control violation: {reason}")]
	FlowControl { reason: String, current_credits: i32, required_credits: i32 },

	#[error("Compression error: {source}")]
	Compression {
		#[from]
		source: CompressionError,
		algorithm: CompressionAlgorithm,
	},

	#[error("Message too large: {size} bytes (max: {max_size})")]
	MessageTooLarge { size: usize, max_size: usize, message_type: MessageType },

	#[error("Invalid frame: {reason}")]
	InvalidFrame { reason: String, frame_data: Option<Bytes> },

	#[error("Connection closed: {code}")]
	ConnectionClosed {
		code: CloseCode,
		reason: Option<String>,
		initiated_by: CloseInitiator,
	},
}

// Error classification for recovery strategies
pub trait ErrorClassification {
	fn is_transient(&self) -> bool;
	fn is_retryable(&self) -> bool;
	fn retry_delay(&self) -> Option<Duration>;
	fn severity(&self) -> ErrorSeverity;
	fn requires_reconnection(&self) -> bool;
}

// Implementation of ErrorClassification trait for TransportError

impl ErrorClassification for TransportError {
	fn is_transient(&self) -> bool {
		match self {
			TransportError::Connection { source, .. } => {
				matches!(
					source,
					ConnectionError::NetworkUnreachable | ConnectionError::ConnectionRefused | ConnectionError::TcpConnection(_)
				)
			}
			TransportError::Timeout { .. } => true,
			TransportError::BufferOverflow { .. } => true,
			TransportError::FlowControl { .. } => true,
			TransportError::NetworkError { .. } => true,
			_ => false,
		}
	}

	fn is_retryable(&self) -> bool {
		match self {
			TransportError::Connection { .. } => true,
			TransportError::Timeout { .. } => true,
			TransportError::BufferOverflow { .. } => false, // Need backpressure handling
			TransportError::FlowControl { .. } => true,
			TransportError::Handshake { .. } => false, // Usually indicates config issues
			TransportError::Protocol { .. } => false,
			TransportError::Tls { .. } => false,
			TransportError::Compression { .. } => false,
			TransportError::MessageTooLarge { .. } => false,
			TransportError::InvalidFrame { .. } => false,
			TransportError::ConnectionClosed { initiated_by, .. } => {
				matches!(initiated_by, CloseInitiator::Remote)
			}
		}
	}

	fn retry_delay(&self) -> Option<Duration> {
		if self.is_retryable() {
			match self {
				TransportError::Connection { retry_after, .. } => *retry_after,
				TransportError::Timeout { .. } => Some(Duration::from_secs(1)),
				TransportError::FlowControl { .. } => Some(Duration::from_millis(100)),
				_ => Some(Duration::from_secs(5)),
			}
		} else {
			None
		}
	}

	fn severity(&self) -> ErrorSeverity {
		match self {
			TransportError::Connection { .. } => ErrorSeverity::Medium,
			TransportError::Handshake { .. } => ErrorSeverity::High,
			TransportError::Protocol { .. } => ErrorSeverity::High,
			TransportError::Tls { .. } => ErrorSeverity::High,
			TransportError::Timeout { .. } => ErrorSeverity::Medium,
			TransportError::BufferOverflow { .. } => ErrorSeverity::High,
			TransportError::FlowControl { .. } => ErrorSeverity::Low,
			TransportError::Compression { .. } => ErrorSeverity::Medium,
			TransportError::MessageTooLarge { .. } => ErrorSeverity::Low,
			TransportError::InvalidFrame { .. } => ErrorSeverity::High,
			TransportError::ConnectionClosed { .. } => ErrorSeverity::Medium,
		}
	}

	fn requires_reconnection(&self) -> bool {
		match self {
			TransportError::Connection { .. } => true,
			TransportError::ConnectionClosed { .. } => true,
			TransportError::Timeout { operation, .. } => operation == "connection",
			TransportError::Tls { .. } => true,
			TransportError::Handshake { .. } => true,
			_ => false,
		}
	}
}

impl TransportError {
	pub fn type_name(&self) -> &'static str {
		match self {
			TransportError::Connection { .. } => "Connection",
			TransportError::Handshake { .. } => "Handshake",
			TransportError::Protocol { .. } => "Protocol",
			TransportError::Tls { .. } => "Tls",
			TransportError::Timeout { .. } => "Timeout",
			TransportError::BufferOverflow { .. } => "BufferOverflow",
			TransportError::FlowControl { .. } => "FlowControl",
			TransportError::Compression { .. } => "Compression",
			TransportError::MessageTooLarge { .. } => "MessageTooLarge",
			TransportError::InvalidFrame { .. } => "InvalidFrame",
			TransportError::ConnectionClosed { .. } => "ConnectionClosed",
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
	#[error("DNS resolution failed")]
	DnsResolution(#[from] std::io::Error),

	#[error("TCP connection failed")]
	TcpConnection(#[from] tokio::io::Error),

	#[error("Network unreachable")]
	NetworkUnreachable,

	#[error("Connection refused")]
	ConnectionRefused,

	#[error("Host unreachable")]
	HostUnreachable,
}

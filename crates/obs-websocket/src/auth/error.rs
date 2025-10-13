#[derive(Debug, Error)]
pub enum AuthTransportError {
	#[error("Send operation failed: {reason}")]
	SendFailed { reason: String },

	#[error("Receive operation failed: {reason}")]
	ReceiveFailed { reason: String },

	#[error("Transport timeout after {duration:?}")]
	Timeout { duration: Duration },

	#[error("Transport disconnected")]
	Disconnected,
}

// Implement From trait to convert transport errors to auth transport errors
impl From<TransportError> for AuthTransportError {
	fn from(error: CoreTransportError) -> Self {
		match error {
			TransportError::Connection { .. } => TransportError::ReceiveFailed {
				reason: "Connection failed".to_string(),
			},
			TransportError::Timeout { duration, .. } => TransportError::Timeout { duration },
			TransportError::ConnectionClosed { reason, .. } => TransportError::Disconnected,
			_ => TransportError::ReceiveFailed {
				reason: format!("Transport error: {}", error),
			},
		}
	}
}

impl From<SendError> for TransportError {
	fn from(error: SendError) -> Self {
		match error {
			SendError::QueueFull => TransportError::SendFailed {
				reason: "Send queue full".to_string(),
			},
			SendError::ConnectionClosed => TransportError::Disconnected,
			SendError::Timeout => TransportError::Timeout {
				duration: std::time::Duration::from_secs(10),
			},
		}
	}
}

/// Authentication-specific errors with detailed context
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum AuthenticationError {
	#[error("Invalid credentials provided")]
	InvalidCredentials,

	#[error("Authentication timeout after {duration:?}")]
	Timeout { duration: Duration },

	#[error("Protocol error: {message}")]
	ProtocolError { message: String, error_code: Option<u16> },

	#[error("Network error during authentication: {details}")]
	NetworkError { details: String },

	#[error("Invalid hello message format: {reason}")]
	InvalidHelloMessage { reason: String },

	#[error("Missing authentication challenge")]
	MissingChallenge,

	#[error("Challenge validation failed")]
	ChallengeValidationFailed,

	#[error("Maximum authentication attempts ({max_attempts}) exceeded")]
	MaxAttemptsExceeded { max_attempts: u32 },

	#[error("Authentication actor unavailable")]
	ActorUnavailable,

	#[error("Invalid session state")]
	InvalidSessionState,
}

// ============================================================================
// Error System - Comprehensive Error Types
// ============================================================================

impl AuthenticationError {
	pub fn is_retryable(&self) -> bool {
		match self {
			Self::InvalidCredentials => false,
			Self::Timeout { .. } => true,
			Self::ProtocolError { .. } => false,
			Self::NetworkError { .. } => true,
			Self::InvalidHelloMessage { .. } => false,
			Self::MissingChallenge => false,
			Self::ChallengeValidationFailed => false,
			Self::MaxAttemptsExceeded { .. } => false,
			Self::ActorUnavailable => true,
			Self::InvalidSessionState => false,
		}
	}

	pub fn retry_delay(&self) -> Option<Duration> {
		if self.is_retryable() {
			Some(Duration::from_secs(1))
		} else {
			None
		}
	}
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ValidationError {
	#[error("Password cannot be empty")]
	EmptyPassword,

	#[error("Timeout cannot be zero")]
	ZeroTimeout,

	#[error("Max attempts cannot be zero")]
	ZeroMaxAttempts,

	#[error("Multiple validation errors: {0:?}")]
	Multiple(Vec<ValidationError>),
}

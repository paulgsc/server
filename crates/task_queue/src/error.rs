use std::num::TryFromIntError;
use std::time::SystemTimeError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum KnownError {
	#[error("Task execution failed: {0}")]
	ExecutionError(String),
	#[error("Task timed out")]
	TimeoutError,
	#[error("Task was cancelled")]
	CancellationError,
	#[error("Queue error: {0}")]
	QueueError(String),
	#[error("Internal error: {0}")]
	InternalError(String),
	#[error("Redis error: {0}")]
	RedisError(#[from] redis::RedisError),
	#[error("Prometheus error: {0}")]
	PrometheusError(#[from] prometheus::Error),
	#[error("JSON error: {0}")]
	JsonError(#[from] serde_json::Error),
	#[error("Conversion error: {0}")]
	ConversionError(String),
}

impl From<SystemTimeError> for KnownError {
	fn from(error: SystemTimeError) -> Self {
		Self::InternalError(format!("System time error: {error}"))
	}
}

impl From<TryFromIntError> for KnownError {
	fn from(error: TryFromIntError) -> Self {
		Self::ConversionError(format!("Conversion error: {error}"))
	}
}

use thiserror::Error;
use std::time::SystemTimeError;

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

}

impl From<SystemTimeError> for KnownError {
    fn from(error: SystemTimeError) -> Self {
        Self::InternalError(format!("System time error: {error}"))
    }
}

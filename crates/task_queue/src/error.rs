use thiserror::Error;

// Error types for task processing
#[derive(Error, Debug)]
pub enum TaskError {
	#[error("Task execution failed: {0}")]
	ExecutionError(String),
	#[error("Task timed out")]
	TimeoutError,
	#[error("Task was cancelled")]
	CancellationError,
	#[error("Queue error: {0}")]
	QueueError(String),
	#[error("Redis error: {0}")]
	RedisError(#[from] redis::RedisError),
}

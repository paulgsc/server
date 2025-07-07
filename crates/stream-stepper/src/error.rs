use thiserror::Error;

pub type Result<T> = std::result::Result<T, ChapterError>;

#[derive(Error, Debug)]
pub enum ChapterError {
	#[error("Serialization error: {0}")]
	Serialization(#[from] serde_json::Error),

	#[error("Timeline error: {0}")]
	Timeline(String),

	#[error("Invalid timestamp: {0}")]
	InvalidTimestamp(String),

	#[error("Event processing error: {0}")]
	EventProcessing(String),

	#[error("State transition error: {0}")]
	StateTransition(String),

	#[error("Chapter not found: {0}")]
	ChapterNotFound(String),
}

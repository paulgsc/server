#![cfg(feature = "stream-orch")]

use thiserror::Error;

pub type Result<T> = std::result::Result<T, OrchestratorError>;

#[derive(Debug, Error)]
pub enum OrchestratorError {
	#[error("Orchestrator not configured")]
	NotConfigured,

	#[error("Orchestrator not running")]
	NotRunning,

	#[error("Orchestrator already running")]
	AlreadyRunning,

	#[error("Scene not found: {0}")]
	SceneNotFound(String),

	#[error("Invalid scene configuration: {0}")]
	InvalidSceneConfig(String),

	#[error("Invalid time: {0}")]
	InvalidTime(String),

	#[error("Schedule conflict: {0}")]
	ScheduleConflict(String),

	#[error("Orchestrator is paused")]
	Paused,

	#[error("Stream not active")]
	StreamNotActive,

	#[error("Internal error: {0}")]
	Internal(String),
}

impl OrchestratorError {
	pub fn is_recoverable(&self) -> bool {
		matches!(self, Self::NotRunning | Self::Paused | Self::StreamNotActive)
	}
}

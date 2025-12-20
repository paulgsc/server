use super::TimeMs;
use serde::{Deserialize, Serialize};

/// Configuration data for a scene (entity/transport type)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneConfigData {
	pub scene_name: String,
	pub duration: TimeMs,
	pub start_time: Option<TimeMs>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub metadata: Option<serde_json::Value>,
}

/// Orchestrator configuration data (entity/transport type)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfigData {
	pub scenes: Vec<SceneConfigData>,
	pub tick_interval_ms: u64,
	pub loop_scenes: bool,
}

/// Commands sent to the orchestrator actor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrchestratorCommandData {
	Configure(OrchestratorConfigData),
	Start,
	Pause,
	Resume,
	Stop,
	Reset,
	ForceScene(String),
	SkipCurrentScene,
	UpdateStreamStatus { is_streaming: bool, stream_time: TimeMs, timecode: String },
}

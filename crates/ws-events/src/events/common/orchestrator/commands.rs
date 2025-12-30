use super::TimeMs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentPlacementData {
	pub registry_key: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub props: Option<serde_json::Value>,
	pub duration: TimeMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusIntentData {
	pub region: String,
	pub intensity: f64, // 0.0 ..= 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelIntentData {
	pub registry_key: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub props: Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub focus: Option<FocusIntentData>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub children: Option<Vec<ComponentPlacementData>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UILayoutIntentData {
	pub panels: HashMap<String, PanelIntentData>,
}

/// Scene configuration entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneConfigData {
	pub scene_name: String,
	pub duration: TimeMs,
	pub start_time: Option<TimeMs>,
	pub ui: Vec<UILayoutIntentData>,
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

use prost::Message;
use serde::{Deserialize, Serialize};
use ws_events::stream_orch::{OrchestratorConfig, SceneConfig};

/// DTO for orchestrator configuration (serializable over NATS)
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct OrchestratorConfigDto {
	#[prost(message, repeated, tag = "1")]
	pub scenes: Vec<SceneConfigDto>,
	#[prost(uint64, optional, tag = "2")]
	pub tick_interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct SceneConfigDto {
	#[prost(string, tag = "1")]
	pub name: String,
	#[prost(uint64, tag = "2")]
	pub duration_ms: u64,
}

impl From<OrchestratorConfigDto> for OrchestratorConfig {
	fn from(dto: OrchestratorConfigDto) -> Self {
		let scenes: Vec<SceneConfig> = dto.scenes.into_iter().map(|s| SceneConfig::new(s.name, s.duration_ms)).collect();

		let mut config = OrchestratorConfig::new(scenes);
		if let Some(interval) = dto.tick_interval_ms {
			config = config.with_tick_interval(interval);
		}
		config
	}
}

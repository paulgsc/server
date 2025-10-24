use prost::Message;
use serde::{Deserialize, Serialize};
use ws_events::stream_orch::{OrchestratorConfig, OrchestratorState, SceneConfig};

mod commands;
mod dto;

/// Serializable state update published to NATS
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct StateUpdate {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(string, optional, tag = "2")]
	pub current_scene: Option<String>,
	#[prost(uint64, tag = "3")]
	pub current_time: u64,
	#[prost(uint64, tag = "4")]
	pub time_remaining: u64,
	#[prost(double, tag = "5")]
	pub progress_percentage: f64,
	#[prost(bool, tag = "6")]
	pub is_running: bool,
	#[prost(bool, tag = "7")]
	pub is_paused: bool,
	#[prost(bool, tag = "8")]
	pub is_complete: bool,
	#[prost(string, tag = "9")]
	pub stream_timecode: String,
	#[prost(bool, tag = "10")]
	pub stream_is_streaming: bool,
}

impl StateUpdate {
	pub fn from_orchestrator_state(stream_id: String, state: &OrchestratorState) -> Self {
		Self {
			stream_id,
			current_scene: state.current_active_scene.clone(),
			current_time: state.current_time,
			time_remaining: state.time_remaining,
			progress_percentage: state.progress.percentage(),
			is_running: state.is_running,
			is_paused: state.is_paused,
			is_complete: state.is_complete(),
			stream_timecode: state.stream_status.timecode.clone(),
			stream_is_streaming: state.stream_status.is_streaming,
		}
	}
}

impl Default for StateUpdate {
	fn default() -> Self {
		Self {
			stream_id: String::new(),
			current_scene: None,
			current_time: 0,
			time_remaining: 0,
			progress_percentage: 0.0,
			is_running: false,
			is_paused: false,
			is_complete: false,
			stream_timecode: String::new(),
			stream_is_streaming: false,
		}
	}
}

/// NATS subject naming conventions
pub mod subjects {
	use super::StreamId;

	/// Command channel for orchestrator control
	pub const ORCHESTRATOR_COMMAND: &str = "orchestrator.command";

	/// Subscription management channel
	pub const ORCHESTRATOR_SUBSCRIPTION: &str = "orchestrator.subscription";

	/// State update channel for a specific stream
	pub fn stream_updates(stream_id: &StreamId) -> String {
		format!("stream.{}.updates", stream_id)
	}
}

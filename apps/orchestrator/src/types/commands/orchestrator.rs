use crate::types::dto::OrchestratorConfigDto;
use prost::Message;
use serde::{Deserialize, Serialize};
use ws_events::stream_orch::SceneConfig;

pub type StreamId = String;
pub type ClientId = String;

/// Commands sent from Axum server to orchestrator service via NATS
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct OrchestratorCommand {
	#[prost(oneof = "orchestrator_command::Command", tags = "1, 2, 3, 4, 5, 6, 7, 8")]
	pub command: Option<Command>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
	#[prost(message, tag = "1")]
	Start(StartCommand),
	#[prost(message, tag = "2")]
	Stop(StopCommand),
	#[prost(message, tag = "3")]
	Pause(PauseCommand),
	#[prost(message, tag = "4")]
	Resume(ResumeCommand),
	#[prost(message, tag = "5")]
	ForceScene(ForceSceneCommand),
	#[prost(message, tag = "6")]
	SkipScene(SkipSceneCommand),
	#[prost(message, tag = "7")]
	UpdateStreamStatus(UpdateStreamStatusCommand),
	#[prost(message, tag = "8")]
	Reconfigure(ReconfigureCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct StartCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(message, tag = "2")]
	pub config: Option<OrchestratorConfigDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct StopCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct PauseCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct ResumeCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct ForceSceneCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(string, tag = "2")]
	pub scene_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct SkipSceneCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct UpdateStreamStatusCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(bool, tag = "2")]
	pub is_streaming: bool,
	#[prost(uint64, tag = "3")]
	pub stream_time: u64,
	#[prost(string, tag = "4")]
	pub timecode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct ReconfigureCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(message, tag = "2")]
	pub config: Option<OrchestratorConfigDto>,
}

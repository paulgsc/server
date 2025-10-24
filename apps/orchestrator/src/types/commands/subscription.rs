use prost::Message;
use serde::{Deserialize, Serialize};
use ws_events::stream_orch::{OrchestratorConfig, OrchestratorState, SceneConfig};

/// Client subscription management commands
#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct SubscriptionCommand {
	#[prost(oneof = "subscription_command::Command", tags = "1, 2, 3")]
	pub command: Option<Command>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
	#[prost(message, tag = "1")]
	Register(RegisterCommand),
	#[prost(message, tag = "2")]
	Unregister(UnregisterCommand),
	#[prost(message, tag = "3")]
	Heartbeat(HeartbeatCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct RegisterCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(string, tag = "2")]
	pub client_id: String,
	#[prost(string, tag = "3")]
	pub source_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct UnregisterCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(string, tag = "2")]
	pub client_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Message)]
pub struct HeartbeatCommand {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(string, tag = "2")]
	pub client_id: String,
}

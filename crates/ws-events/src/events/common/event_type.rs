use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
	ObsStatus,
	ObsCommand,
	ClientCount,
	Ping,
	Pong,
	Error,
	TabMetaData,
	Utterance,
	OrchestratorCommandData,
	OrchestratorState,
	SystemEvent,
	AudioChunk,
	Subtitle,
}

impl Default for EventType {
	fn default() -> Self {
		EventType::Pong
	}
}

impl fmt::Display for EventType {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.subject())
	}
}

impl EventType {
	/// Get the NATS subject prefix for this event type
	pub fn subject(&self) -> &'static str {
		match self {
			EventType::ObsStatus => "obs.status",
			EventType::ObsCommand => "obs.command",
			EventType::TabMetaData => "tab.metadata",
			EventType::ClientCount => "system.client_count",
			EventType::Error => "system.error",
			EventType::Utterance => "utterance",
			EventType::OrchestratorCommandData => "orchestrator.command",
			EventType::OrchestratorState => "orchestrator.state",
			EventType::SystemEvent => "system",
			EventType::AudioChunk => "audio.chunk",
			EventType::Subtitle => "audio.subtitle",
			// These don't have subjects as they're not transported
			EventType::Ping | EventType::Pong => "system.ping",
		}
	}

	/// Get the connection-specific subject for this event type
	pub fn connection_subject(&self, connection_id: &str) -> String {
		format!("{}.{}", self.subject(), connection_id)
	}

	/// Check if this event type should be transported via NATS
	pub fn is_transportable(&self) -> bool {
		!matches!(self, EventType::Ping | EventType::Pong)
	}
}

impl std::str::FromStr for EventType {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"obs.status" => Ok(EventType::ObsStatus),
			"obs.command" => Ok(EventType::ObsCommand),
			"tab.metadata" => Ok(EventType::TabMetaData),
			"system.client_count" => Ok(EventType::ClientCount),
			"system.error" => Ok(EventType::Error),
			"utterance" => Ok(EventType::Utterance),
			"system.ping" => Ok(EventType::Ping),
			"obsStatus" => Ok(EventType::ObsStatus),
			"obsCommand" => Ok(EventType::ObsCommand),
			"clientCount" => Ok(EventType::ClientCount),
			"ping" => Ok(EventType::Ping),
			"pong" => Ok(EventType::Pong),
			"error" => Ok(EventType::Error),
			"tabMetaData" => Ok(EventType::TabMetaData),
			"orchestrator.command" => Ok(EventType::OrchestratorCommandData),
			"orchestrator.state" => Ok(EventType::OrchestratorState),
			"systemEvent" => Ok(EventType::SystemEvent),
			_ => Err(format!("Unknown event type: {}", s)),
		}
	}
}

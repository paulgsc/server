use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum EventType {
	ObsStatus,
	ObsCommand,
	ClientCount,
	Ping,
	#[default]
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

impl fmt::Display for EventType {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.subject())
	}
}

impl EventType {
	/// Get the NATS subject prefix for this event type
	#[must_use]
	pub const fn subject(&self) -> &'static str {
		match self {
			Self::ObsStatus => "obs.status",
			Self::ObsCommand => "obs.command",
			Self::TabMetaData => "tab.metadata",
			Self::ClientCount => "system.client_count",
			Self::Error => "system.error",
			Self::Utterance => "utterance",
			Self::OrchestratorCommandData => "orchestrator.command",
			Self::OrchestratorState => "orchestrator.state",
			Self::SystemEvent => "system",
			Self::AudioChunk => "audio.chunk",
			Self::Subtitle => "audio.subtitle",
			// These don't have subjects as they're not transported
			Self::Ping | Self::Pong => "system.ping",
		}
	}

	/// Get the connection-specific subject for this event type
	#[must_use]
	pub fn connection_subject(&self, connection_id: &str) -> String {
		owned_format!("{}.{}", self.subject(), connection_id)
	}

	/// Check if this event type should be transported via NATS
	#[must_use]
	pub const fn is_transportable(&self) -> bool {
		!matches!(self, Self::Ping | Self::Pong)
	}
}

impl std::str::FromStr for EventType {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"obs.status" | "obsStatus" => Ok(Self::ObsStatus),
			"obs.command" | "obsCommand" => Ok(Self::ObsCommand),
			"tab.metadata" | "tabMetaData" => Ok(Self::TabMetaData),
			"system.client_count" | "clientCount" => Ok(Self::ClientCount),
			"system.error" | "error" => Ok(Self::Error),
			"utterance" => Ok(Self::Utterance),
			"system.ping" | "ping" => Ok(Self::Ping),
			"pong" => Ok(Self::Pong),
			"orchestrator.command" => Ok(Self::OrchestratorCommandData),
			"orchestrator.state" => Ok(Self::OrchestratorState),
			"systemEvent" => Ok(Self::SystemEvent),
			_ => Err(owned_format!("Unknown event type: {s}")),
		}
	}
}

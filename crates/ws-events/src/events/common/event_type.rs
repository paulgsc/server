use serde::{Deserialize, Serialize};
use std::fmt;

/// The NATS subjects' epoch, as home's release states it.
///
/// RELEASE R3, #790; docs/release.md, "Constraints". `orchestrator`'s image provides
/// `nats-subjects=<EPOCH>` and `file_host`'s requires it, both as
/// `io.paulgsc.*` labels in their Dockerfiles, which each side's own test
/// holds to this number.
///
/// It covers what crosses these subjects between the two, the subjects
/// and the payloads (`UnifiedEvent`, `OrchestratorCommandData`,
/// `OrchestratorState`) alike, though only the subjects are pinned by a
/// test: raise it by hand with a payload change the other side must read.
/// Raise it by one whenever [`EventType::subject`] changes:
/// `the_subjects_are_this_epoch_s` (in `apps/orchestrator/src/lib.rs`, since
/// this crate runs no tests) fails until the subjects it pins and this
/// number move together.
pub const NATS_SUBJECTS_EPOCH: u32 = 1;

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

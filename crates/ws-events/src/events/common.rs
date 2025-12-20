use serde::{Deserialize, Serialize};

mod event_type;
mod message;
mod now_playing;
mod orchestrator;
mod system_events;
mod utterance;

pub use event_type::EventType;
pub use message::{MessageId, ProcessResult};
pub use now_playing::NowPlaying;
use obs_websocket::{ObsCommand, ObsEvent};
pub use orchestrator::{
	ActiveLifetime, LifetimeEvent, LifetimeId, LifetimeKind, OrchestratorCommandData, OrchestratorConfigData, OrchestratorEvent, OrchestratorState, Progress, SceneConfigData,
	SceneId, ScenePayload, StreamStatus, TimeMs, TimedEvent,
};
pub use system_events::SystemEvent;
pub use utterance::{UtteranceMetadata, UtterancePrompt};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Event {
	ObsStatus {
		status: ObsEvent,
	},
	ObsCmd {
		cmd: ObsCommand,
	},
	TabMetaData {
		data: NowPlaying,
	},
	ClientCount {
		count: usize,
	},
	Ping,
	Pong,
	Error {
		message: String,
	},
	Subscribe {
		event_types: Vec<EventType>,
	},
	Unsubscribe {
		event_types: Vec<EventType>,
	},
	Utterance {
		text: String,
		metadata: UtteranceMetadata,
	},
	#[serde(skip)]
	System(SystemEvent),
	OrchestratorCommandData {
		stream_id: String,
		command: OrchestratorCommandData,
	},
	OrchestratorState {
		stream_id: String,
		state: OrchestratorState,
	},
	AudioChunk {
		sample_rate: u32,
		channels: u32,
		samples: Vec<f32>,
	},
	Subtitle {
		text: String,
		timestamp: u64,
		confidence: Option<f32>,
	},
}

impl Event {
	pub fn get_type(&self) -> Option<EventType> {
		match self {
			Self::Ping => Some(EventType::Ping),
			Self::Pong => Some(EventType::Pong),
			Self::Error { .. } => Some(EventType::Error),
			Self::Subscribe { .. } => Some(EventType::Ping), // These are control messages
			Self::Unsubscribe { .. } => Some(EventType::Ping),
			Self::ClientCount { .. } => Some(EventType::ClientCount),
			Self::ObsStatus { .. } => Some(EventType::ObsStatus),
			Self::ObsCmd { .. } => Some(EventType::ObsCommand),
			Self::TabMetaData { .. } => Some(EventType::TabMetaData),
			Self::Utterance { .. } => Some(EventType::Utterance),
			Self::OrchestratorCommandData { .. } => Some(EventType::OrchestratorCommandData),
			Self::OrchestratorState { .. } => Some(EventType::OrchestratorState),
			Self::AudioChunk { .. } => Some(EventType::AudioChunk),
			Self::Subtitle { .. } => Some(EventType::Subtitle),
			// System events don't have EventTypes
			_ => None,
		}
	}
}

impl From<NowPlaying> for Event {
	fn from(data: NowPlaying) -> Self {
		Event::TabMetaData { data }
	}
}

impl From<UtterancePrompt> for Event {
	fn from(UtterancePrompt { text, metadata }: UtterancePrompt) -> Self {
		Event::Utterance { text, metadata }
	}
}

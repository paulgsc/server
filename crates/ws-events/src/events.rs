#![cfg(feature = "ws-events")]

mod common;
mod unified;

pub use common::OrchestratorMode;
pub use common::{ActiveLifetime, LifetimeEvent, LifetimeId, LifetimeKind, OrchestratorEvent, Progress, StreamStatus, TimedEvent};
pub use common::{ComponentPlacementData, FocusIntentData, OrchestratorCommandData, PanelIntentData};
pub use common::{Event, EventType, MessageId, NowPlaying, OrchestratorState, ProcessResult, UtteranceMetadata, UtterancePrompt};
pub use common::{OrchestratorConfigData, SceneConfigData, SceneId, ScenePayload, SystemEvent, TimeMs, UILayoutIntentData};
pub use unified::unified_event;
pub use unified::UnifiedEvent;
pub use unified::{AudioChunkMessage, ObsCommandMessage, ObsStatusMessage, SubtitleMessage};

pub(crate) fn to_json_vec<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
	let mut bytes = Vec::new();
	serde_json::to_writer(&mut bytes, value)?;
	Ok(bytes)
}

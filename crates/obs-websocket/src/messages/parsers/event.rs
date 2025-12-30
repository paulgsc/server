use crate::messages::{JsonExtractor, ObsMessagesError};
use crate::types::*;
use serde_json::Value;
use tracing::{trace, warn};

type Result<T> = std::result::Result<T, ObsMessagesError>;

/// Handles parsing of OBS events (op: 5)
pub(crate) struct EventMessageParser;

impl EventMessageParser {
	pub fn parse(json: &Value) -> Result<ObsEvent> {
		let extractor = JsonExtractor::new(json, "Event message");
		let d = extractor.get_object("d")?;

		let val = Value::Object(d.clone());
		let d_extractor = JsonExtractor::new(&val, "Event data");
		let event_type_str = d_extractor.get_string("eventType")?;
		let event_type = ObsEventType::from_str(event_type_str);

		trace!("Parsing event type: {}", event_type_str);

		let event = match event_type {
			ObsEventType::StreamStateChanged => Self::parse_stream_state(d)?,
			ObsEventType::RecordStateChanged => Self::parse_record_state(d)?,
			ObsEventType::CurrentProgramSceneChanged => Self::parse_scene_change(d)?,
			ObsEventType::SceneItemEnableStateChanged => Self::parse_scene_item_state(d)?,
			ObsEventType::InputMuteStateChanged => Self::parse_input_mute_state(d)?,
			ObsEventType::InputVolumeChanged => Self::parse_input_volume(d)?,
			ObsEventType::VirtualcamStateChanged => Self::parse_virtualcam_state(d)?,
			ObsEventType::ReplayBufferStateChanged => Self::parse_replay_buffer_state(d)?,
			ObsEventType::StudioModeStateChanged => Self::parse_studio_mode_state(d)?,
			ObsEventType::CurrentSceneTransitionChanged => Self::parse_transition_change(d)?,
			ObsEventType::SceneTransitionStarted => Self::parse_transition_started(d)?,
			ObsEventType::SceneTransitionEnded => Self::parse_transition_ended(d)?,
			_ => {
				warn!("Unknown event type: {}, creating UnknownEvent", event_type_str);
				ObsEvent::UnknownEvent(UnknownEventData {
					event_type: event_type_str.to_string(),
					data: Value::Object(d.clone()),
				})
			}
		};

		trace!("Successfully parsed event: {:?}", event);
		Ok(event)
	}

	fn parse_stream_state(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let streaming = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
		let timecode = if streaming {
			d.get("outputTimecode").and_then(Value::as_str).map(String::from)
		} else {
			Some("00:00:00.000".to_string())
		};
		Ok(ObsEvent::StreamStateChanged(StreamStateData { streaming, timecode }))
	}

	fn parse_record_state(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let recording = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
		let timecode = if recording {
			d.get("outputTimecode").and_then(Value::as_str).map(String::from)
		} else {
			Some("00:00:00.000".to_string())
		};
		Ok(ObsEvent::RecordStateChanged(RecordStateData { recording, timecode }))
	}

	fn parse_scene_change(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let val = Value::Object(d.clone());
		let extractor = JsonExtractor::new(&val, "CurrentProgramSceneChanged");
		let scene_name = extractor.get_string("sceneName")?.to_string();
		Ok(ObsEvent::CurrentProgramSceneChanged(CurrentProgramSceneData { scene_name }))
	}

	fn parse_scene_item_state(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let scene_name = d.get("sceneName").and_then(Value::as_str).unwrap_or("").to_string();
		let item_id = d.get("sceneItemId").and_then(Value::as_u64).unwrap_or(0) as u32;
		let enabled = d.get("sceneItemEnabled").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::SceneItemEnableStateChanged(SceneItemEnableStateData { scene_name, item_id, enabled }))
	}

	fn parse_input_mute_state(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let input_name = d.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
		let muted = d.get("inputMuted").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::InputMuteStateChanged(InputMuteStateData { input_name, muted }))
	}

	fn parse_input_volume(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let input_name = d.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
		let volume_db = d.get("inputVolumeDb").and_then(Value::as_f64).unwrap_or(0.0);
		let volume_mul = d.get("inputVolumeMul").and_then(Value::as_f64).unwrap_or(1.0);
		Ok(ObsEvent::InputVolumeChanged(InputVolumeData {
			input_name,
			volume_db,
			volume_mul,
		}))
	}

	fn parse_virtualcam_state(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let active = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::VirtualcamStateChanged(VirtualcamStateData { active }))
	}

	fn parse_replay_buffer_state(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let active = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::ReplayBufferStateChanged(ReplayBufferStateData { active }))
	}

	fn parse_studio_mode_state(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let enabled = d.get("studioModeEnabled").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::StudioModeStateChanged(StudioModeStateData { enabled }))
	}

	fn parse_transition_change(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::CurrentSceneTransitionChanged(CurrentSceneTransitionData { transition_name }))
	}

	fn parse_transition_started(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::SceneTransitionStarted(SceneTransitionStartedData { transition_name }))
	}

	fn parse_transition_ended(d: &serde_json::Map<String, Value>) -> Result<ObsEvent> {
		let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::SceneTransitionEnded(SceneTransitionEndedData { transition_name }))
	}
}

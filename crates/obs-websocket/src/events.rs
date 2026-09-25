//! Maps `obws` events onto this crate's wire type, [`ObsEvent`].

use crate::types::{
	CurrentProgramSceneData, CurrentSceneTransitionData, InputMuteStateData, InputVolumeData, ObsEvent, RecordStateData, ReplayBufferStateData, SceneItemEnableStateData,
	SceneTransitionEndedData, SceneTransitionStartedData, StreamStateData, StudioModeStateData, UnknownEventData, VirtualcamStateData,
};
use obws::events::{Event, OutputState};
use serde_json::Value;

const STOPPED_TIMECODE: &str = "00:00:00.000";

/// Converts an `obws` event. Events without a dedicated [`ObsEvent`] variant
/// become [`ObsEvent::UnknownEvent`] carrying OBS's `eventType`/`eventData`;
/// `None` only for events `obws` itself could not identify, which carry no data.
pub fn to_obs_event(event: Event) -> Option<ObsEvent> {
	let mapped = match event {
		Event::StreamStateChanged { active, state } => ObsEvent::StreamStateChanged(StreamStateData {
			streaming: active,
			// OBS sends no timecode with state changes; polling supplies it.
			timecode: stopped_timecode(active),
			output_state: output_state_name(state),
		}),
		Event::RecordStateChanged { active, state, .. } => ObsEvent::RecordStateChanged(RecordStateData {
			recording: active,
			timecode: stopped_timecode(active),
			output_state: output_state_name(state),
		}),
		Event::CurrentProgramSceneChanged { id } => ObsEvent::CurrentProgramSceneChanged(CurrentProgramSceneData { scene_name: id.name }),
		Event::SceneItemEnableStateChanged { scene, item_id, enabled } => ObsEvent::SceneItemEnableStateChanged(SceneItemEnableStateData {
			scene_name: scene.name,
			item_id: u32::try_from(item_id).unwrap_or(u32::MAX),
			enabled,
		}),
		Event::InputMuteStateChanged { id, muted } => ObsEvent::InputMuteStateChanged(InputMuteStateData { input_name: id.name, muted }),
		Event::InputVolumeChanged { id, mul, db } => ObsEvent::InputVolumeChanged(InputVolumeData {
			input_name: id.name,
			volume_db: db,
			volume_mul: mul,
		}),
		Event::VirtualcamStateChanged { active, .. } => ObsEvent::VirtualcamStateChanged(VirtualcamStateData { active }),
		Event::ReplayBufferStateChanged { active, .. } => ObsEvent::ReplayBufferStateChanged(ReplayBufferStateData { active }),
		Event::StudioModeStateChanged { enabled } => ObsEvent::StudioModeStateChanged(StudioModeStateData { enabled }),
		Event::CurrentSceneTransitionChanged { id } => ObsEvent::CurrentSceneTransitionChanged(CurrentSceneTransitionData { transition_name: id.name }),
		Event::SceneTransitionStarted { id } => ObsEvent::SceneTransitionStarted(SceneTransitionStartedData { transition_name: id.name }),
		Event::SceneTransitionEnded { id } => ObsEvent::SceneTransitionEnded(SceneTransitionEndedData { transition_name: id.name }),
		Event::Unknown => return None,
		other => unknown_event(&other)?,
	};
	Some(mapped)
}

fn stopped_timecode(active: bool) -> Option<String> {
	(!active).then(|| STOPPED_TIMECODE.to_owned())
}

/// OBS's own name for the state, e.g. `OBS_WEBSOCKET_OUTPUT_STARTED`.
fn output_state_name(state: OutputState) -> Option<String> {
	match serde_json::to_value(state) {
		Ok(Value::String(name)) => Some(name),
		_ => None,
	}
}

/// Re-serializes an event into OBS's `{eventType, eventData}` shape.
fn unknown_event(event: &Event) -> Option<ObsEvent> {
	let data = serde_json::to_value(event).ok()?;
	let event_type = data.get("eventType")?.as_str()?.to_owned();
	Some(ObsEvent::UnknownEvent(UnknownEventData { event_type, data }))
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	/// Builds an `obws` event from the JSON obs-websocket puts in an event's `d`.
	fn event(d: &Value) -> Event {
		serde_json::from_value(d.clone()).unwrap()
	}

	#[test]
	fn stream_state_carries_obs_output_state() {
		let mapped = to_obs_event(event(&json!({
			"eventType": "StreamStateChanged",
			"eventData": { "outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED" }
		})));

		match mapped {
			Some(ObsEvent::StreamStateChanged(data)) => {
				assert!(data.streaming);
				assert_eq!(data.timecode, None);
				assert_eq!(data.output_state.as_deref(), Some("OBS_WEBSOCKET_OUTPUT_STARTED"));
			}
			other => panic!("expected StreamStateChanged, got {other:?}"),
		}
	}

	#[test]
	fn stopped_stream_reports_zero_timecode() {
		let mapped = to_obs_event(event(&json!({
			"eventType": "StreamStateChanged",
			"eventData": { "outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED" }
		})));

		assert!(matches!(
			mapped,
			Some(ObsEvent::StreamStateChanged(StreamStateData { streaming: false, timecode: Some(ref t), .. })) if t == STOPPED_TIMECODE
		));
	}

	#[test]
	fn scene_change_maps_the_scene_name() {
		let mapped = to_obs_event(event(&json!({
			"eventType": "CurrentProgramSceneChanged",
			"eventData": { "sceneName": "Live", "sceneUuid": "7a9f3c1e-0000-4000-8000-000000000001" }
		})));

		assert!(matches!(mapped, Some(ObsEvent::CurrentProgramSceneChanged(data)) if data.scene_name == "Live"));
	}

	#[test]
	fn unmodelled_events_are_forwarded_with_their_data() {
		let mapped = to_obs_event(event(&json!({
			"eventType": "CurrentProfileChanged",
			"eventData": { "profileName": "Streaming" }
		})));

		match mapped {
			Some(ObsEvent::UnknownEvent(data)) => {
				assert_eq!(data.event_type, "CurrentProfileChanged");
				assert_eq!(data.data["eventData"]["profileName"], "Streaming");
			}
			other => panic!("expected UnknownEvent, got {other:?}"),
		}
	}
}

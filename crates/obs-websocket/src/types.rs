use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ObsCommand {
	StartStream,
	StopStream,
	StartRecording,
	StopRecording,
	SwitchScene(String),
	SetInputMute(String, bool),
	SetInputVolume(String, f64),
	ToggleStudioMode(bool),
	StartVirtualCamera,
	StopVirtualCamera,
	StartReplayBuffer,
	StopReplayBuffer,
	GetInputMute(String),
	GetInputVolume(String),
	/// Point OBS's stream output at the `YouTube` RTMP ingest using `stream_key`.
	///
	/// This only configures where OBS pushes video. A broadcast's title,
	/// description and privacy live on the platform side and are set through
	/// the `YouTube` Data API: OBS's `rtmp_custom` service ignores such fields.
	SetYouTubeStream {
		stream_key: StreamKey,
	},
	/// No longer supported: `obws` exposes only typed requests. Kept so older
	/// payloads still deserialize and get an explicit error reply; add a typed
	/// variant for whatever request this was carrying.
	Custom(Value),
}

/// A stream key: anyone holding one can publish to the channel it belongs to,
/// so `Debug` redacts it rather than letting it reach logs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamKey(String);

impl StreamKey {
	#[must_use]
	pub const fn new(key: String) -> Self {
		Self(key)
	}

	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Debug for StreamKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("StreamKey(***)")
	}
}

/// Represents different types of events from OBS
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ObsEvent {
	// Stream and Recording Status
	StreamStatusResponse(StreamStatusData),
	RecordingStatusResponse(RecordingStatusData),

	// Scene Management
	SceneListResponse(SceneListData),
	CurrentSceneResponse(CurrentSceneData),

	// Source Management
	SourcesListResponse(SourcesListData),
	InputListResponse(InputListData),

	// Audio Management
	AudioMuteResponse(AudioMuteData),
	AudioVolumeResponse(AudioVolumeData),

	// Profile and Collection Management
	ProfileListResponse(ProfileListData),
	CurrentProfileResponse(CurrentProfileData),
	SceneCollectionListResponse(SceneCollectionListData),
	CurrentCollectionResponse(CurrentCollectionData),

	// Virtual Camera
	VirtualCamStatusResponse(VirtualCamStatusData),

	// Replay Buffer
	ReplayBufferStatusResponse(ReplayBufferStatusData),

	// Studio Mode
	StudioModeResponse(StudioModeData),

	// Statistics
	StatsResponse(StatsData),

	// Transitions
	CurrentTransitionResponse(CurrentTransitionData),
	TransitionListResponse(TransitionListData),

	// Filters
	FilterListResponse(FilterListData),

	// Hotkeys
	HotkeyListResponse(HotkeyListData),

	// Version
	VersionResponse(VersionData),

	// Real-time events (op: 5)
	StreamStateChanged(StreamStateData),
	RecordStateChanged(RecordStateData),
	CurrentProgramSceneChanged(CurrentProgramSceneData),
	SceneItemEnableStateChanged(SceneItemEnableStateData),
	InputMuteStateChanged(InputMuteStateData),
	InputVolumeChanged(InputVolumeData),
	VirtualcamStateChanged(VirtualcamStateData),
	ReplayBufferStateChanged(ReplayBufferStateData),
	StudioModeStateChanged(StudioModeStateData),
	CurrentSceneTransitionChanged(CurrentSceneTransitionData),
	SceneTransitionStarted(SceneTransitionStartedData),
	SceneTransitionEnded(SceneTransitionEndedData),

	// Generic events for unhandled cases
	UnknownResponse(UnknownResponseData),
	UnknownEvent(UnknownEventData),
}

// Data structures for each enum variant
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatusData {
	pub streaming: bool,
	pub timecode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatusData {
	pub recording: bool,
	pub timecode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneListData {
	pub scenes: Vec<SceneInfo>,
	pub current_scene: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSceneData {
	pub scene_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcesListData {
	pub sources: Vec<SourceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputListData {
	pub inputs: Vec<InputInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioMuteData {
	pub input_name: String,
	pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioVolumeData {
	pub input_name: String,
	pub volume_db: f64,
	pub volume_mul: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileListData {
	pub profiles: Vec<String>,
	pub current_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentProfileData {
	pub profile_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneCollectionListData {
	pub collections: Vec<String>,
	pub current_collection: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentCollectionData {
	pub collection_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualCamStatusData {
	pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayBufferStatusData {
	pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioModeData {
	pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsData {
	pub stats: ObsStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentTransitionData {
	pub transition_name: String,
	pub transition_duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionListData {
	pub transitions: Vec<TransitionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterListData {
	pub source_name: String,
	pub filters: Vec<FilterInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyListData {
	pub hotkeys: Vec<HotkeyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionData {
	pub obs_version: String,
	pub websocket_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStateData {
	pub streaming: bool,
	pub timecode: Option<String>,
	/// OBS's `outputState`, e.g. `OBS_WEBSOCKET_OUTPUT_STARTED`. A successful
	/// `StartStream` response only means OBS began starting the output; this
	/// is where a failed ingest connection shows up (`..._STOPPED`).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub output_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordStateData {
	pub recording: bool,
	pub timecode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentProgramSceneData {
	pub scene_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneItemEnableStateData {
	pub scene_name: String,
	pub item_id: u32,
	pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputMuteStateData {
	pub input_name: String,
	pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputVolumeData {
	pub input_name: String,
	pub volume_db: f64,
	pub volume_mul: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualcamStateData {
	pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayBufferStateData {
	pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioModeStateData {
	pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSceneTransitionData {
	pub transition_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneTransitionStartedData {
	pub transition_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneTransitionEndedData {
	pub transition_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnknownResponseData {
	pub request_type: String,
	pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnknownEventData {
	pub event_type: String,
	pub data: Value,
}

/// Scene information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneInfo {
	pub name: String,
	pub index: Option<u32>,
}

/// Source information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
	pub name: String,
	pub type_id: String,
	pub kind: String,
}

/// Input information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputInfo {
	pub name: String,
	pub kind: String,
	pub unversioned_kind: String,
}

/// Transition information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionInfo {
	pub name: String,
	pub kind: String,
	pub fixed: bool,
}

/// Filter information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterInfo {
	pub name: String,
	pub kind: String,
	pub index: u32,
	pub enabled: bool,
}

/// Hotkey information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyInfo {
	pub name: String,
	pub description: String,
}

/// OBS Statistics structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsStats {
	pub cpu_usage: f64,
	pub memory_usage: f64,
	pub available_disk_space: f64,
	pub active_fps: f64,
	pub average_frame_time: f64,
	pub render_total_frames: u64,
	pub render_missed_frames: u64,
	pub output_total_frames: u64,
	pub output_skipped_frames: u64,
	pub web_socket_session_incoming_messages: u64,
	pub web_socket_session_outgoing_messages: u64,
}

impl Default for ObsStats {
	fn default() -> Self {
		Self {
			cpu_usage: 0.0,
			memory_usage: 0.0,
			available_disk_space: 0.0,
			active_fps: 0.0,
			average_frame_time: 0.0,
			render_total_frames: 0,
			render_missed_frames: 0,
			output_total_frames: 0,
			output_skipped_frames: 0,
			web_socket_session_incoming_messages: 0,
			web_socket_session_outgoing_messages: 0,
		}
	}
}

impl ObsEvent {
	/// Check if this event should trigger a status broadcast
	#[must_use]
	pub const fn should_broadcast(&self) -> bool {
		matches!(
			self,
			Self::StreamStatusResponse(_)
				| Self::RecordingStatusResponse(_)
				| Self::SceneListResponse(_)
				| Self::CurrentSceneResponse(_)
				| Self::VirtualCamStatusResponse(_)
				| Self::ReplayBufferStatusResponse(_)
				| Self::StudioModeResponse(_)
				| Self::StreamStateChanged(_)
				| Self::RecordStateChanged(_)
				| Self::CurrentProgramSceneChanged(_)
				| Self::VirtualcamStateChanged(_)
				| Self::ReplayBufferStateChanged(_)
				| Self::StudioModeStateChanged(_)
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn set_youtube_stream_accepts_the_old_payload_shape() {
		// Senders built against the earlier variant still send metadata fields;
		// they must keep deserializing, with the extras ignored.
		let command: ObsCommand = serde_json::from_value(json!({
			"type": "setYouTubeStream",
			"data": {
				"stream_key": "abcd-efgh",
				"title": "t", "description": "d", "category": "c",
				"privacy": "public", "unlisted": false, "tags": []
			}
		}))
		.unwrap();

		match command {
			ObsCommand::SetYouTubeStream { stream_key } => assert_eq!(stream_key.expose(), "abcd-efgh"),
			other => panic!("expected SetYouTubeStream, got {other:?}"),
		}
	}

	#[test]
	fn stream_key_is_redacted_in_debug_output() {
		let command = ObsCommand::SetYouTubeStream {
			stream_key: StreamKey::new("abcd-efgh".to_owned()),
		};
		let rendered = std::fmt::format(format_args!("{command:?}"));
		assert!(!rendered.contains("abcd-efgh"), "{rendered}");
	}
}

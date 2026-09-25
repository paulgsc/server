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
	SetYouTubeStream { stream_key: StreamKey },
	/// A raw single request (`op: 6`). A missing `d.requestId` is filled in so
	/// the response can be matched back to the caller.
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

/// Represents different types of requests that can be sent to OBS
#[derive(Debug, Clone, Serialize)]
pub enum ObsRequestType {
	// General
	GetVersion,
	GetStats,
	GetHotkeyList,

	// Scene Management
	GetSceneList,
	GetCurrentProgramScene,
	SetCurrentProgramScene,
	GetSceneTransitionList,
	GetCurrentSceneTransition,
	#[allow(dead_code)]
	SetCurrrentSceneTransition,
	#[allow(dead_code)]
	TriggerSceneTransition,

	// Source Management
	GetInputList,
	GetInputMute,
	SetInputMute,
	GetInputVolume,
	SetInputVolume,
	GetSourceFilterList,
	GetSourcesList,

	// Stream and Recording Status
	GetStreamStatus,
	GetRecordStatus,
	StartStream,
	StopStream,
	StartRecord,
	StopRecord,

	// Profile and Collection Management
	GetProfileList,
	GetCurrentProfile,
	SetCurrentProfile,
	GetSceneCollectionList,
	GetCurrentSceneCollection,
	SetCurrentSceneCollection,

	// Virtual Camera
	GetVirtualCamStatus,
	StartVirtualCam,
	StopVirtualCam,

	// Replay Buffer
	GetReplayBufferStatus,
	StartReplayBuffer,
	StopReplayBuffer,

	// Studio Mode
	GetStudioModeEnabled,
	SetStudioModeEnabled,

	// Filters
	#[allow(dead_code)]
	GetSourceFilter,
	#[allow(dead_code)]
	SetSourceFilterEnabled,

	// Service Management
	SetStreamServiceSettings,
	GetStreamServiceSettings,

	// Unknown/Unhandled
	Unknown(String),
}

impl ObsRequestType {
	#[must_use]
	pub fn as_str(&self) -> &str {
		match self {
			Self::GetVersion => "GetVersion",
			Self::GetStats => "GetStats",
			Self::GetHotkeyList => "GetHotkeyList",
			Self::GetSceneList => "GetSceneList",
			Self::GetCurrentProgramScene => "GetCurrentProgramScene",
			Self::SetCurrentProgramScene => "SetCurrentProgramScene",
			Self::GetSceneTransitionList => "GetSceneTransitionList",
			Self::GetCurrentSceneTransition => "GetCurrentSceneTransition",
			Self::SetCurrrentSceneTransition => "SetCurrrentSceneTransition",
			Self::TriggerSceneTransition => "TriggerSceneTransition",
			Self::GetInputList => "GetInputList",
			Self::GetInputMute => "GetInputMute",
			Self::GetInputVolume => "GetInputVolume",
			Self::SetInputMute => "SetInputMute",
			Self::SetInputVolume => "SetInputVolume",
			Self::GetSourceFilterList => "GetSourceFilterList",
			Self::GetSourcesList => "GetSourcesList",
			Self::GetStreamStatus => "GetStreamStatus",
			Self::GetRecordStatus => "GetRecordStatus",
			Self::StartStream => "StartStream",
			Self::StopStream => "StopStream",
			Self::StartRecord => "StartRecord",
			Self::StopRecord => "StopRecord",
			Self::GetProfileList => "GetProfileList",
			Self::GetCurrentProfile => "GetCurrentProfile",
			Self::SetCurrentProfile => "SetCurrentProfile",
			Self::GetSceneCollectionList => "GetSceneCollectionList",
			Self::GetCurrentSceneCollection => "GetCurrentSceneCollection",
			Self::SetCurrentSceneCollection => "SetCurrentSceneCollection",
			Self::GetVirtualCamStatus => "GetVirtualCamStatus",
			Self::StartVirtualCam => "StartVirtualCam",
			Self::StopVirtualCam => "StopVirtualCam",
			Self::GetReplayBufferStatus => "GetReplayBufferStatus",
			Self::StartReplayBuffer => "StartReplayBuffer",
			Self::StopReplayBuffer => "StopReplayBuffer",
			Self::GetStudioModeEnabled => "GetStudioModeEnabled",
			Self::SetStudioModeEnabled => "SetStudioModeEnabled",
			Self::GetSourceFilter => "GetSourceFilter",
			Self::SetSourceFilterEnabled => "SetSourceFilterEnabled",
			Self::SetStreamServiceSettings => "SetStreamServiceSettings",
			Self::GetStreamServiceSettings => "GetStreamServiceSettings",
			Self::Unknown(s) => s,
		}
	}
}

impl From<&str> for ObsRequestType {
	/// Maps an OBS request type name to its variant, falling back to
	/// [`Self::Unknown`] for names this crate does not model.
	fn from(s: &str) -> Self {
		match s {
			"GetStreamStatus" => Self::GetStreamStatus,
			"GetRecordStatus" => Self::GetRecordStatus,
			"StartStream" => Self::StartStream,
			"StopStream" => Self::StopStream,
			"StartRecord" => Self::StartRecord,
			"StopRecord" => Self::StopRecord,
			"GetSceneList" => Self::GetSceneList,
			"GetCurrentProgramScene" => Self::GetCurrentProgramScene,
			"SetCurrentProgramScene" => Self::SetCurrentProgramScene,
			"GetSourcesList" => Self::GetSourcesList,
			"GetInputList" => Self::GetInputList,
			"GetInputMute" => Self::GetInputMute,
			"SetInputMute" => Self::SetInputMute,
			"GetInputVolume" => Self::GetInputVolume,
			"SetInputVolume" => Self::SetInputVolume,
			"GetProfileList" => Self::GetProfileList,
			"GetCurrentProfile" => Self::GetCurrentProfile,
			"SetCurrentProfile" => Self::SetCurrentProfile,
			"GetSceneCollectionList" => Self::GetSceneCollectionList,
			"GetCurrentSceneCollection" => Self::GetCurrentSceneCollection,
			"SetCurrentSceneCollection" => Self::SetCurrentSceneCollection,
			"GetVirtualCamStatus" => Self::GetVirtualCamStatus,
			"StartVirtualCam" => Self::StartVirtualCam,
			"StopVirtualCam" => Self::StopVirtualCam,
			"GetReplayBufferStatus" => Self::GetReplayBufferStatus,
			"StartReplayBuffer" => Self::StartReplayBuffer,
			"StopReplayBuffer" => Self::StopReplayBuffer,
			"GetStudioModeEnabled" => Self::GetStudioModeEnabled,
			"SetStudioModeEnabled" => Self::SetStudioModeEnabled,
			"GetStats" => Self::GetStats,
			"GetCurrentSceneTransition" => Self::GetCurrentSceneTransition,
			// "SetCurrentSceneTransition" => Self::SetCurrentSceneTransition,
			"GetSceneTransitionList" => Self::GetSceneTransitionList,
			"GetSourceFilterList" => Self::GetSourceFilterList,
			"GetHotkeyList" => Self::GetHotkeyList,
			"GetVersion" => Self::GetVersion,
			"SetStreamServiceSettings" => Self::SetStreamServiceSettings,
			"GetStreamServiceSettings" => Self::GetStreamServiceSettings,
			unknown => Self::Unknown(unknown.to_string()),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObsEventType {
	// Stream events
	StreamStateChanged,

	// Recording events
	RecordStateChanged,

	// Scene events
	CurrentProgramSceneChanged,
	SceneItemEnableStateChanged,

	// Input events
	InputMuteStateChanged,
	InputVolumeChanged,

	// Virtual camera events
	VirtualcamStateChanged,

	// Replay buffer events
	ReplayBufferStateChanged,

	// Studio mode events
	StudioModeStateChanged,

	// Transition events
	CurrentSceneTransitionChanged,
	SceneTransitionStarted,
	SceneTransitionEnded,

	// Unknown/Unhandled
	Unknown(String),
}

impl ObsEventType {
	#[allow(dead_code)]
	#[must_use]
	pub fn as_str(&self) -> &str {
		match self {
			Self::StreamStateChanged => "StreamStateChanged",
			Self::RecordStateChanged => "RecordStateChanged",
			Self::CurrentProgramSceneChanged => "CurrentProgramSceneChanged",
			Self::SceneItemEnableStateChanged => "SceneItemEnableStateChanged",
			Self::InputMuteStateChanged => "InputMuteStateChanged",
			Self::InputVolumeChanged => "InputVolumeChanged",
			Self::VirtualcamStateChanged => "VirtualcamStateChanged",
			Self::ReplayBufferStateChanged => "ReplayBufferStateChanged",
			Self::StudioModeStateChanged => "StudioModeStateChanged",
			Self::CurrentSceneTransitionChanged => "CurrentSceneTransitionChanged",
			Self::SceneTransitionStarted => "SceneTransitionStarted",
			Self::SceneTransitionEnded => "SceneTransitionEnded",
			Self::Unknown(s) => s,
		}
	}
}

impl From<&str> for ObsEventType {
	/// Maps an OBS event type name to its variant, falling back to
	/// [`Self::Unknown`] for names this crate does not model.
	fn from(s: &str) -> Self {
		match s {
			"StreamStateChanged" => Self::StreamStateChanged,
			"RecordStateChanged" => Self::RecordStateChanged,
			"CurrentProgramSceneChanged" => Self::CurrentProgramSceneChanged,
			"SceneItemEnableStateChanged" => Self::SceneItemEnableStateChanged,
			"InputMuteStateChanged" => Self::InputMuteStateChanged,
			"InputVolumeChanged" => Self::InputVolumeChanged,
			"VirtualcamStateChanged" => Self::VirtualcamStateChanged,
			"ReplayBufferStateChanged" => Self::ReplayBufferStateChanged,
			"StudioModeStateChanged" => Self::StudioModeStateChanged,
			"CurrentSceneTransitionChanged" => Self::CurrentSceneTransitionChanged,
			"SceneTransitionStarted" => Self::SceneTransitionStarted,
			"SceneTransitionEnded" => Self::SceneTransitionEnded,
			unknown => Self::Unknown(unknown.to_string()),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct ObsRequest<T>
where
	T: Serialize,
{
	#[serde(rename = "op")]
	pub op_code: u8,
	pub d: RequestData<T>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestData<T>
where
	T: Serialize,
{
	#[serde(rename = "requestType")]
	pub t: ObsRequestType,
	#[serde(rename = "requestId")]
	pub id: String,
	#[serde(rename = "requestData")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub p: Option<T>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetCurrentProgramSceneParams {
	#[serde(rename = "sceneName")]
	pub scene_name: String,
}

// Request field names must match obs-websocket v5's protocol exactly: OBS
// answers an unrecognised name with `MissingRequestField` (code 300).

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputNameParams {
	pub input_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetInputMuteParams {
	pub input_name: String,
	pub input_muted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetInputVolumeParams {
	pub input_name: String,
	/// Linear multiplier, `0.0..=20.0`, where `1.0` is unity gain.
	pub input_volume_mul: f64,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct SetCurrrentSceneTransitionParams {
	pub n: String,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct SetCurrentProfileParams {
	pub n: String,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct SetCurrentSceneCollectionParams {
	pub n: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetStudioModeEnabledParams {
	pub studio_mode_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct SetSourceFilterEnabledParams {
	pub n: String,
	pub f: String,
	pub b: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetStreamServiceSettingsParams {
	#[serde(rename = "streamServiceType")]
	pub service_type: String,
	#[serde(rename = "streamServiceSettings")]
	pub settings: StreamServiceSettings,
}

/// Settings for OBS's `rtmp_custom` stream service, which reads only these two
/// fields (plus optional auth this crate doesn't use).
#[derive(Debug, Clone, Serialize)]
pub struct StreamServiceSettings {
	pub server: String,
	pub key: StreamKey,
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

	// Connection events
	Hello(HelloData),
	Identified,

	// Service Management
	StreamServiceSettingsResponse(StreamServiceSettingsData),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloData {
	pub obs_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamServiceSettingsData {
	pub stream_service_type: String,
	pub stream_service_settings: StreamServiceSettingsResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamServiceSettingsResponse {
	pub key: Option<String>,
	pub server: Option<String>,
	pub title: Option<String>,
	pub description: Option<String>,
	pub game: Option<String>,
	pub privacy: Option<String>,
	pub unlisted: Option<bool>,
	pub tags: Option<Vec<String>>,
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

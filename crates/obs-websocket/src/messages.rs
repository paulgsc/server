use futures_util::{sink::SinkExt, stream::SplitSink};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream};
use tracing::{debug, error, info, instrument, trace, warn};

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

	// Unknown/Unhandled
	Unknown(String),
}

impl ObsRequestType {
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
			Self::Unknown(s) => s,
		}
	}

	pub fn from_str(s: &str) -> Self {
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

	pub fn from_str(s: &str) -> Self {
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
	pub t: ObsRequestType,
	pub id: String,
	#[serde(flatten)]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub p: Option<T>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetCurrentProgramSceneParams {
	pub n: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetInputMuteParams {
	pub n: String,
	pub b: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetInputVolumeParams {
	pub n: String,
	pub v: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetCurrrentSceneTransitionParams {
	pub n: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetCurrentProfileParams {
	pub n: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetCurrentSceneCollectionParams {
	pub n: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetStudioModeEnabledParams {
	pub b: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetSourceFilterEnabledParams {
	pub n: String,
	pub f: String,
	pub b: bool,
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
	pub const fn should_broadcast(&self) -> bool {
		match self {
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
			| Self::StudioModeStateChanged(_) => true,
			_ => false,
		}
	}
}

/// Wait for hello message from OBS and send initial state requests
pub async fn fetch_init_state(
	sink: &mut SplitSink<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
	let requests = [
		(ObsRequestType::GetSceneList, "scenes-init"),
		(ObsRequestType::GetStreamStatus, "stream-init"),
		(ObsRequestType::GetRecordStatus, "recording-init"),
		(ObsRequestType::GetCurrentProgramScene, "current-scene-init"),
		(ObsRequestType::GetVirtualCamStatus, "vcam-init"),
		(ObsRequestType::GetStudioModeEnabled, "studio-init"),
	];

	for (request_type, request_id) in requests {
		let request = json!({
			"op": 6,
			"d": {
				"requestType": request_type.as_str(),
				"requestId": request_id
			}
		});

		debug!("Requesting initial {}: {}", request_type.as_str(), request);
		sink.send(TungsteniteMessage::Text(request.to_string().into())).await?;
	}

	sink.flush().await?;
	info!("Sent all initial state requests");

	Ok(())
}

/// Parse OBS message and return appropriate event
#[instrument(skip(text), fields(message_len = text.len()))]
pub async fn process_obs_message(text: String) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	trace!("Starting to process OBS message of {} bytes", text.len());

	// Parse JSON with detailed error handling
	let json: Value = match serde_json::from_str(&text) {
		Ok(json) => {
			trace!("Successfully parsed JSON from OBS message");
			json
		}
		Err(e) => {
			error!("Failed to parse JSON from OBS message: {}", e);
			return Err(format!("JSON parse error: {e}").into());
		}
	};

	// Extract op code with detailed logging
	let op = match json.get("op") {
		Some(op_value) => {
			match op_value.as_u64() {
				Some(op_code) => {
					debug!("Extracted op code: {}", op_code);
					op_code
				}
				None => {
					error!("Invalid op field in message: {}", text);
					99 // Default fallback
				}
			}
		}
		None => {
			error!("Missing op field in message: {}", text);
			99 // Default fallback
		}
	};

	debug!("Processing OBS message with op: {}", op);

	let event = match op {
		0 => {
			info!("Processing Hello message (op=0)");
			match process_hello_message(&json) {
				Ok(event) => {
					info!("Successfully processed Hello message");
					event
				}
				Err(e) => {
					error!("Failed to process Hello message: {}", e);
					return Err(e);
				}
			}
		}
		2 => {
			trace!("Identified message JSON: {}", json);
			ObsEvent::Identified
		}
		5 => {
			debug!("Processing OBS Event message (op=5)");
			match parse_obs_event(&json) {
				Ok(event) => {
					debug!("Successfully parsed OBS event: {:?}", event);
					event
				}
				Err(e) => {
					error!("Failed to parse OBS event: {}", e);
					return Err(format!("OBS event parse error: {}", e).into());
				}
			}
		}
		7 => {
			debug!("Processing OBS Response message (op=7)");
			match parse_obs_response(&json) {
				Ok(response) => {
					debug!("Successfully parsed OBS response: {:?}", response);
					response
				}
				Err(e) => {
					error!("Failed to parse OBS response: {}", e);
					return Err(format!("OBS response parse error: {}", e).into());
				}
			}
		}
		_ => {
			error!("Unknown op code {} in message: {}", op, text);
			return Err(format!("Unknown op code: {}", op).into());
		}
	};

	trace!("Successfully processed OBS message, returning event: {:?}", event);
	Ok(event)
}

/// Helper function to process Hello messages with detailed tracing
#[instrument(skip(json))]
fn process_hello_message(json: &Value) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	trace!("Extracting 'd' field from Hello message");

	let d = match json.get("d") {
		Some(d_value) => match d_value.as_object() {
			Some(d_obj) => {
				trace!("Found 'd' object with {} fields", d_obj.len());
				d_obj
			}
			None => {
				error!("'d' field exists but is not an object: {:?}", d_value);
				return Err("'d' field is not an object in Hello message".into());
			}
		},
		None => {
			error!("Missing 'd' field in Hello message");
			return Err("Missing 'd' field in Hello message".into());
		}
	};

	trace!("Extracting obsWebSocketVersion from Hello message");
	let obs_version = match d.get("obsWebSocketVersion") {
		Some(version_value) => match version_value.as_str() {
			Some(version_str) => {
				info!("Found OBS WebSocket version: {}", version_str);
				version_str.to_string()
			}
			None => {
				warn!("obsWebSocketVersion exists but is not a string: {:?}", version_value);
				warn!("Using 'unknown' as fallback version");
				"unknown".to_string()
			}
		},
		None => {
			warn!("No obsWebSocketVersion field found in Hello message");
			warn!("Using 'unknown' as fallback version");
			"unknown".to_string()
		}
	};

	let hello_data = HelloData { obs_version };
	info!("Created Hello event with version: {}", hello_data.obs_version);
	Ok(ObsEvent::Hello(hello_data))
}

/// Parse OBS event messages (op: 5)
fn parse_obs_event(json: &Value) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	let d = json.get("d").and_then(Value::as_object).ok_or("Missing 'd' field in event message")?;
	let e = d.get("eventType").and_then(Value::as_str).ok_or("Missing eventType in event message")?;
	let e_t = ObsEventType::from_str(e);

	let event = match e_t {
		ObsEventType::StreamStateChanged => {
			let streaming = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = if streaming {
				d.get("outputTimecode").and_then(Value::as_str).map(String::from)
			} else {
				Some("00:00:00.000".to_string())
			};
			ObsEvent::StreamStateChanged(StreamStateData { streaming, timecode })
		}
		ObsEventType::RecordStateChanged => {
			let recording = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = if recording {
				d.get("outputTimecode").and_then(Value::as_str).map(String::from)
			} else {
				Some("00:00:00.000".to_string())
			};
			ObsEvent::RecordStateChanged(RecordStateData { recording, timecode })
		}
		ObsEventType::CurrentProgramSceneChanged => {
			let scene_name = d
				.get("sceneName")
				.and_then(Value::as_str)
				.ok_or("Missing sceneName in CurrentProgramSceneChanged event")?
				.to_string();
			ObsEvent::CurrentProgramSceneChanged(CurrentProgramSceneData { scene_name })
		}
		ObsEventType::SceneItemEnableStateChanged => {
			let scene_name = d.get("sceneName").and_then(Value::as_str).unwrap_or("").to_string();
			let item_id = d.get("sceneItemId").and_then(Value::as_u64).unwrap_or(0) as u32;
			let enabled = d.get("sceneItemEnabled").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::SceneItemEnableStateChanged(SceneItemEnableStateData { scene_name, item_id, enabled })
		}
		ObsEventType::InputMuteStateChanged => {
			let input_name = d.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
			let muted = d.get("inputMuted").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::InputMuteStateChanged(InputMuteStateData { input_name, muted })
		}
		ObsEventType::InputVolumeChanged => {
			let input_name = d.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
			let volume_db = d.get("inputVolumeDb").and_then(Value::as_f64).unwrap_or(0.0);
			let volume_mul = d.get("inputVolumeMul").and_then(Value::as_f64).unwrap_or(1.0);
			ObsEvent::InputVolumeChanged(InputVolumeData {
				input_name,
				volume_db,
				volume_mul,
			})
		}
		ObsEventType::VirtualcamStateChanged => {
			let active = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::VirtualcamStateChanged(VirtualcamStateData { active })
		}
		ObsEventType::ReplayBufferStateChanged => {
			let active = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::ReplayBufferStateChanged(ReplayBufferStateData { active })
		}
		ObsEventType::StudioModeStateChanged => {
			let enabled = d.get("studioModeEnabled").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::StudioModeStateChanged(StudioModeStateData { enabled })
		}
		ObsEventType::CurrentSceneTransitionChanged => {
			let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::CurrentSceneTransitionChanged(CurrentSceneTransitionData { transition_name })
		}
		ObsEventType::SceneTransitionStarted => {
			let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::SceneTransitionStarted(SceneTransitionStartedData { transition_name })
		}
		ObsEventType::SceneTransitionEnded => {
			let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::SceneTransitionEnded(SceneTransitionEndedData { transition_name })
		}
		_ => {
			debug!("Unhandled event type: {:?}", e_t);
			ObsEvent::UnknownEvent(UnknownEventData {
				event_type: e_t.as_str().into(),
				data: d.clone().into(),
			})
		}
	};

	Ok(event)
}

/// Parse OBS response messages (op: 7)
fn parse_obs_response(json: &Value) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	let d = json.get("d").and_then(Value::as_object).ok_or("Missing 'd' field in response message")?;
	let r = d.get("requestType").and_then(Value::as_str).ok_or("Missing requestType in response message")?;
	let r_t = ObsRequestType::from_str(r);

	let request_status = d.get("requestStatus").and_then(Value::as_object);
	if let Some(status) = request_status {
		let success = status.get("result").and_then(Value::as_bool).unwrap_or(false);
		if !success {
			let code = status.get("code").and_then(Value::as_u64).unwrap_or(0);
			let comment = status.get("comment").and_then(Value::as_str).unwrap_or("Unknown error");

			warn!("OBS request failed - Type: {:?}, Code: {}, Comment: {}", r_t, code, comment);

			// Return an error event for failed requests
			return Ok(ObsEvent::UnknownResponse(UnknownResponseData {
				request_type: r_t.as_str().into(),
				data: json!({
					"error": true,
					"code": code,
					"comment": comment
				}),
			}));
		}
	}

	let response_data = d.get("responseData");
	if response_data.is_none() {
		debug!("No responseData in successful response for: {:?}", r_t);
		return Ok(ObsEvent::UnknownResponse(UnknownResponseData {
			request_type: r_t.as_str().into(),
			data: json!({"success": true, "no_data": true}),
		}));
	}

	let response_data = response_data.ok_or("Missing responseData in response message")?;
	let event = match r_t {
		ObsRequestType::GetStreamStatus => {
			let streaming = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = response_data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();
			ObsEvent::StreamStatusResponse(StreamStatusData { streaming, timecode })
		}
		ObsRequestType::GetRecordStatus => {
			let recording = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = response_data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();
			ObsEvent::RecordingStatusResponse(RecordingStatusData { recording, timecode })
		}
		ObsRequestType::GetSceneList => {
			let scenes = response_data
				.get("scenes")
				.and_then(Value::as_array)
				.map(|arr| {
					arr
						.iter()
						.filter_map(|s| {
							Some(SceneInfo {
								name: s.get("sceneName").and_then(Value::as_str)?.to_string(),
								index: s.get("sceneIndex").and_then(Value::as_u64).map(|i| i as u32),
							})
						})
						.collect()
				})
				.unwrap_or_default();
			let current_scene = response_data.get("currentProgramSceneName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::SceneListResponse(SceneListData { scenes, current_scene })
		}
		ObsRequestType::GetCurrentProgramScene => {
			let scene_name = response_data.get("sceneName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::CurrentSceneResponse(CurrentSceneData { scene_name })
		}
		ObsRequestType::GetSourcesList => {
			let sources = response_data
				.get("sources")
				.and_then(Value::as_array)
				.map(|arr| {
					arr
						.iter()
						.filter_map(|s| {
							Some(SourceInfo {
								name: s.get("sourceName").and_then(Value::as_str)?.to_string(),
								type_id: s.get("sourceType").and_then(Value::as_str).unwrap_or("").to_string(),
								kind: s.get("sourceKind").and_then(Value::as_str).unwrap_or("").to_string(),
							})
						})
						.collect()
				})
				.unwrap_or_default();
			ObsEvent::SourcesListResponse(SourcesListData { sources })
		}
		ObsRequestType::GetInputList => {
			let inputs = response_data
				.get("inputs")
				.and_then(Value::as_array)
				.map(|arr| {
					arr
						.iter()
						.filter_map(|i| {
							Some(InputInfo {
								name: i.get("inputName").and_then(Value::as_str)?.to_string(),
								kind: i.get("inputKind").and_then(Value::as_str).unwrap_or("").to_string(),
								unversioned_kind: i.get("unversionedInputKind").and_then(Value::as_str).unwrap_or("").to_string(),
							})
						})
						.collect()
				})
				.unwrap_or_default();
			ObsEvent::InputListResponse(InputListData { inputs })
		}
		ObsRequestType::GetInputMute => {
			let input_name = response_data.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
			let muted = response_data.get("inputMuted").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::AudioMuteResponse(AudioMuteData { input_name, muted })
		}
		ObsRequestType::GetInputVolume => {
			let input_name = response_data.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
			let volume_db = response_data.get("inputVolumeDb").and_then(Value::as_f64).unwrap_or(0.0);
			let volume_mul = response_data.get("inputVolumeMul").and_then(Value::as_f64).unwrap_or(1.0);
			ObsEvent::AudioVolumeResponse(AudioVolumeData {
				input_name,
				volume_db,
				volume_mul,
			})
		}
		ObsRequestType::GetProfileList => {
			let profiles = response_data
				.get("profiles")
				.and_then(Value::as_array)
				.map(|arr| arr.iter().filter_map(|p| p.as_str().map(String::from)).collect())
				.unwrap_or_default();
			let current_profile = response_data.get("currentProfileName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::ProfileListResponse(ProfileListData { profiles, current_profile })
		}
		ObsRequestType::GetCurrentProfile => {
			let profile_name = response_data.get("profileName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::CurrentProfileResponse(CurrentProfileData { profile_name })
		}
		ObsRequestType::GetSceneCollectionList => {
			let collections = response_data
				.get("sceneCollections")
				.and_then(Value::as_array)
				.map(|arr| arr.iter().filter_map(|c| c.as_str().map(String::from)).collect())
				.unwrap_or_default();
			let current_collection = response_data.get("currentSceneCollectionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::SceneCollectionListResponse(SceneCollectionListData { collections, current_collection })
		}
		ObsRequestType::GetCurrentSceneCollection => {
			let collection_name = response_data.get("sceneCollectionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::CurrentCollectionResponse(CurrentCollectionData { collection_name })
		}
		ObsRequestType::GetVirtualCamStatus => {
			let active = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::VirtualCamStatusResponse(VirtualCamStatusData { active })
		}
		ObsRequestType::GetReplayBufferStatus => {
			let active = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::ReplayBufferStatusResponse(ReplayBufferStatusData { active })
		}
		ObsRequestType::GetStudioModeEnabled => {
			let enabled = response_data.get("studioModeEnabled").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::StudioModeResponse(StudioModeData { enabled })
		}
		ObsRequestType::GetStats => {
			let stats = ObsStats {
				cpu_usage: response_data.get("cpuUsage").and_then(Value::as_f64).unwrap_or(0.0),
				memory_usage: response_data.get("memoryUsage").and_then(Value::as_f64).unwrap_or(0.0),
				available_disk_space: response_data.get("availableDiskSpace").and_then(Value::as_f64).unwrap_or(0.0),
				active_fps: response_data.get("activeFps").and_then(Value::as_f64).unwrap_or(0.0),
				average_frame_time: response_data.get("averageFrameTime").and_then(Value::as_f64).unwrap_or(0.0),
				render_total_frames: response_data.get("renderTotalFrames").and_then(Value::as_u64).unwrap_or(0),
				render_missed_frames: response_data.get("renderMissedFrames").and_then(Value::as_u64).unwrap_or(0),
				output_total_frames: response_data.get("outputTotalFrames").and_then(Value::as_u64).unwrap_or(0),
				output_skipped_frames: response_data.get("outputSkippedFrames").and_then(Value::as_u64).unwrap_or(0),
				web_socket_session_incoming_messages: response_data.get("webSocketSessionIncomingMessages").and_then(Value::as_u64).unwrap_or(0),
				web_socket_session_outgoing_messages: response_data.get("webSocketSessionOutgoingMessages").and_then(Value::as_u64).unwrap_or(0),
			};
			ObsEvent::StatsResponse(StatsData { stats })
		}
		ObsRequestType::GetCurrentSceneTransition => {
			let transition_name = response_data.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
			let transition_duration = response_data.get("transitionDuration").and_then(Value::as_u64).unwrap_or(0) as u32;
			ObsEvent::CurrentTransitionResponse(CurrentTransitionData {
				transition_name,
				transition_duration,
			})
		}
		ObsRequestType::GetSceneTransitionList => {
			let transitions = response_data
				.get("transitions")
				.and_then(Value::as_array)
				.map(|arr| {
					arr
						.iter()
						.filter_map(|t| {
							Some(TransitionInfo {
								name: t.get("transitionName").and_then(Value::as_str)?.to_string(),
								kind: t.get("transitionKind").and_then(Value::as_str).unwrap_or("").to_string(),
								fixed: t.get("transitionFixed").and_then(Value::as_bool).unwrap_or(false),
							})
						})
						.collect()
				})
				.unwrap_or_default();
			ObsEvent::TransitionListResponse(TransitionListData { transitions })
		}
		ObsRequestType::GetSourceFilterList => {
			let source_name = response_data.get("sourceName").and_then(Value::as_str).unwrap_or("").to_string();
			let filters = response_data
				.get("filters")
				.and_then(Value::as_array)
				.map(|arr| {
					arr
						.iter()
						.filter_map(|f| {
							Some(FilterInfo {
								name: f.get("filterName").and_then(Value::as_str)?.to_string(),
								kind: f.get("filterKind").and_then(Value::as_str).unwrap_or("").to_string(),
								index: f.get("filterIndex").and_then(Value::as_u64).unwrap_or(0) as u32,
								enabled: f.get("filterEnabled").and_then(Value::as_bool).unwrap_or(false),
							})
						})
						.collect()
				})
				.unwrap_or_default();
			ObsEvent::FilterListResponse(FilterListData { source_name, filters })
		}
		ObsRequestType::GetHotkeyList => {
			let hotkeys = response_data
				.get("hotkeys")
				.and_then(Value::as_array)
				.map(|arr| {
					arr
						.iter()
						.filter_map(|h| {
							Some(HotkeyInfo {
								name: h.get("hotkeyName").and_then(Value::as_str)?.to_string(),
								description: h.get("hotkeyDescription").and_then(Value::as_str).unwrap_or("").to_string(),
							})
						})
						.collect()
				})
				.unwrap_or_default();
			ObsEvent::HotkeyListResponse(HotkeyListData { hotkeys })
		}
		ObsRequestType::GetVersion => {
			let obs_version = response_data.get("obsVersion").and_then(Value::as_str).unwrap_or("").to_string();
			let websocket_version = response_data.get("obsWebSocketVersion").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::VersionResponse(VersionData { obs_version, websocket_version })
		}
		_ => {
			debug!("Unhandled response type: {:?}", r_t);
			ObsEvent::UnknownResponse(UnknownResponseData {
				request_type: r_t.as_str().into(),
				data: response_data.clone(),
			})
		}
	};

	Ok(event)
}

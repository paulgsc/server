use futures_util::{sink::SinkExt, stream::SplitSink};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream};
use tracing::{debug, info, warn};

/// Represents different types of events from OBS
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ObsEvent {
	// Stream and Recording Status
	StreamStatusResponse { streaming: bool, timecode: String },
	RecordingStatusResponse { recording: bool, timecode: String },

	// Scene Management
	SceneListResponse { scenes: Vec<SceneInfo>, current_scene: String },
	CurrentSceneResponse { scene_name: String },

	// Source Management
	SourcesListResponse { sources: Vec<SourceInfo> },
	InputListResponse { inputs: Vec<InputInfo> },

	// Audio Management
	AudioMuteResponse { input_name: String, muted: bool },
	AudioVolumeResponse { input_name: String, volume_db: f64, volume_mul: f64 },

	// Profile and Collection Management
	ProfileListResponse { profiles: Vec<String>, current_profile: String },
	CurrentProfileResponse { profile_name: String },
	SceneCollectionListResponse { collections: Vec<String>, current_collection: String },
	CurrentCollectionResponse { collection_name: String },

	// Virtual Camera
	VirtualCamStatusResponse { active: bool },

	// Replay Buffer
	ReplayBufferStatusResponse { active: bool },

	// Studio Mode
	StudioModeResponse { enabled: bool },

	// Statistics
	StatsResponse { stats: ObsStats },

	// Transitions
	CurrentTransitionResponse { transition_name: String, transition_duration: u32 },
	TransitionListResponse { transitions: Vec<TransitionInfo> },

	// Filters
	FilterListResponse { source_name: String, filters: Vec<FilterInfo> },

	// Hotkeys
	HotkeyListResponse { hotkeys: Vec<HotkeyInfo> },

	// Version
	VersionResponse { obs_version: String, websocket_version: String },

	// Real-time events (op: 5)
	StreamStateChanged { streaming: bool, timecode: Option<String> },
	RecordStateChanged { recording: bool, timecode: Option<String> },
	CurrentProgramSceneChanged { scene_name: String },
	SceneItemEnableStateChanged { scene_name: String, item_id: u32, enabled: bool },
	InputMuteStateChanged { input_name: String, muted: bool },
	InputVolumeChanged { input_name: String, volume_db: f64, volume_mul: f64 },
	VirtualcamStateChanged { active: bool },
	ReplayBufferStateChanged { active: bool },
	StudioModeStateChanged { enabled: bool },
	CurrentSceneTransitionChanged { transition_name: String },
	SceneTransitionStarted { transition_name: String },
	SceneTransitionEnded { transition_name: String },

	// Generic events for unhandled cases
	UnknownResponse { request_type: String, data: Value },
	UnknownEvent { event_type: String, data: Value },

	// Connection events
	Hello { obs_version: String },
	Identified,
}

/// Scene information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneInfo {
	pub name: String,
	pub index: Option<u32>,
}

/// Source information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
	pub name: String,
	pub type_id: String,
	pub kind: String,
}

/// Input information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputInfo {
	pub name: String,
	pub kind: String,
	pub unversioned_kind: String,
}

/// Transition information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionInfo {
	pub name: String,
	pub kind: String,
	pub fixed: bool,
}

/// Filter information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterInfo {
	pub name: String,
	pub kind: String,
	pub index: u32,
	pub enabled: bool,
}

/// Hotkey information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyInfo {
	pub name: String,
	pub description: String,
}

/// OBS Statistics structure
#[derive(Debug, Clone, Serialize, Deserialize)]
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
			Self::StreamStatusResponse { .. }
			| Self::RecordingStatusResponse { .. }
			| Self::SceneListResponse { .. }
			| Self::CurrentSceneResponse { .. }
			| Self::VirtualCamStatusResponse { .. }
			| Self::ReplayBufferStatusResponse { .. }
			| Self::StudioModeResponse { .. }
			| Self::StreamStateChanged { .. }
			| Self::RecordStateChanged { .. }
			| Self::CurrentProgramSceneChanged { .. }
			| Self::VirtualcamStateChanged { .. }
			| Self::ReplayBufferStateChanged { .. }
			| Self::StudioModeStateChanged { .. } => true,
			_ => false,
		}
	}
}

/// Wait for hello message from OBS and send initial state requests
pub async fn fetch_init_state(
	sink: &mut SplitSink<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
	let requests = [
		("GetSceneList", "scenes-init"),
		("GetStreamStatus", "stream-init"),
		("GetRecordStatus", "recording-init"),
		("GetCurrentProgramScene", "current-scene-init"),
		("GetVirtualCamStatus", "vcam-init"),
		("GetStudioModeEnabled", "studio-init"),
	];

	for (request_type, request_id) in requests {
		let request = json!({
			"op": 6,
			"d": {
				"requestType": request_type,
				"requestId": request_id
			}
		});

		debug!("Requesting initial {}: {}", request_type, request);
		sink.send(TungsteniteMessage::Text(request.to_string().into())).await?;
	}

	sink.flush().await?;
	info!("Sent all initial state requests");

	Ok(())
}

/// Parse OBS message and return appropriate event
pub async fn process_obs_message(text: String) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	let json: Value = serde_json::from_str(&text)?;
	let op = json.get("op").and_then(Value::as_u64).unwrap_or(99);

	debug!("Processing OBS message with op: {}", op);

	let event = match op {
		0 => {
			// Hello message
			let d = json.get("d").and_then(Value::as_object).ok_or("Missing 'd' field in Hello message")?;
			let obs_version = d.get("obsWebSocketVersion").and_then(Value::as_str).unwrap_or("unknown").to_string();

			ObsEvent::Hello { obs_version }
		}
		2 => {
			// Identified
			ObsEvent::Identified
		}
		5 => {
			// Event from OBS
			parse_obs_event(&json)?
		}
		7 => {
			// Request response from OBS
			parse_obs_response(&json)?
		}
		_ => {
			warn!("Unknown OBS operation code: {}", op);
			return Err(format!("Unknown op code: {}", op).into());
		}
	};

	Ok(event)
}

/// Parse OBS event messages (op: 5)
fn parse_obs_event(json: &Value) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	let d = json.get("d").and_then(Value::as_object).ok_or("Missing 'd' field in event message")?;
	let event_type = d.get("eventType").and_then(Value::as_str).ok_or("Missing eventType in event message")?;

	let event = match event_type {
		"StreamStateChanged" => {
			let streaming = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = if streaming {
				d.get("outputTimecode").and_then(Value::as_str).map(String::from)
			} else {
				Some("00:00:00.000".to_string())
			};
			ObsEvent::StreamStateChanged { streaming, timecode }
		}
		"RecordStateChanged" => {
			let recording = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = if recording {
				d.get("outputTimecode").and_then(Value::as_str).map(String::from)
			} else {
				Some("00:00:00.000".to_string())
			};
			ObsEvent::RecordStateChanged { recording, timecode }
		}
		"CurrentProgramSceneChanged" => {
			let scene_name = d
				.get("sceneName")
				.and_then(Value::as_str)
				.ok_or("Missing sceneName in CurrentProgramSceneChanged event")?
				.to_string();
			ObsEvent::CurrentProgramSceneChanged { scene_name }
		}
		"SceneItemEnableStateChanged" => {
			let scene_name = d.get("sceneName").and_then(Value::as_str).unwrap_or("").to_string();
			let item_id = d.get("sceneItemId").and_then(Value::as_u64).unwrap_or(0) as u32;
			let enabled = d.get("sceneItemEnabled").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::SceneItemEnableStateChanged { scene_name, item_id, enabled }
		}
		"InputMuteStateChanged" => {
			let input_name = d.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
			let muted = d.get("inputMuted").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::InputMuteStateChanged { input_name, muted }
		}
		"InputVolumeChanged" => {
			let input_name = d.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
			let volume_db = d.get("inputVolumeDb").and_then(Value::as_f64).unwrap_or(0.0);
			let volume_mul = d.get("inputVolumeMul").and_then(Value::as_f64).unwrap_or(1.0);
			ObsEvent::InputVolumeChanged {
				input_name,
				volume_db,
				volume_mul,
			}
		}
		"VirtualcamStateChanged" => {
			let active = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::VirtualcamStateChanged { active }
		}
		"ReplayBufferStateChanged" => {
			let active = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::ReplayBufferStateChanged { active }
		}
		"StudioModeStateChanged" => {
			let enabled = d.get("studioModeEnabled").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::StudioModeStateChanged { enabled }
		}
		"CurrentSceneTransitionChanged" => {
			let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::CurrentSceneTransitionChanged { transition_name }
		}
		"SceneTransitionStarted" => {
			let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::SceneTransitionStarted { transition_name }
		}
		"SceneTransitionEnded" => {
			let transition_name = d.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::SceneTransitionEnded { transition_name }
		}
		_ => {
			debug!("Unhandled event type: {}", event_type);
			ObsEvent::UnknownEvent {
				event_type: event_type.to_string(),
				data: d.clone().into(),
			}
		}
	};

	Ok(event)
}

/// Parse OBS response messages (op: 7)
fn parse_obs_response(json: &Value) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	let d = json.get("d").and_then(Value::as_object).ok_or("Missing 'd' field in response message")?;
	let request_type = d.get("requestType").and_then(Value::as_str).ok_or("Missing requestType in response message")?;
	let response_data = d.get("responseData").ok_or("Missing responseData in response message")?;

	let event = match request_type {
		"GetStreamStatus" => {
			let streaming = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = response_data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();
			ObsEvent::StreamStatusResponse { streaming, timecode }
		}
		"GetRecordStatus" => {
			let recording = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = response_data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();
			ObsEvent::RecordingStatusResponse { recording, timecode }
		}
		"GetSceneList" => {
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
			ObsEvent::SceneListResponse { scenes, current_scene }
		}
		"GetCurrentProgramScene" => {
			let scene_name = response_data.get("sceneName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::CurrentSceneResponse { scene_name }
		}
		"GetSourcesList" => {
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
			ObsEvent::SourcesListResponse { sources }
		}
		"GetInputList" => {
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
			ObsEvent::InputListResponse { inputs }
		}
		"GetInputMute" => {
			let input_name = response_data.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
			let muted = response_data.get("inputMuted").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::AudioMuteResponse { input_name, muted }
		}
		"GetInputVolume" => {
			let input_name = response_data.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
			let volume_db = response_data.get("inputVolumeDb").and_then(Value::as_f64).unwrap_or(0.0);
			let volume_mul = response_data.get("inputVolumeMul").and_then(Value::as_f64).unwrap_or(1.0);
			ObsEvent::AudioVolumeResponse {
				input_name,
				volume_db,
				volume_mul,
			}
		}
		"GetProfileList" => {
			let profiles = response_data
				.get("profiles")
				.and_then(Value::as_array)
				.map(|arr| arr.iter().filter_map(|p| p.as_str().map(String::from)).collect())
				.unwrap_or_default();
			let current_profile = response_data.get("currentProfileName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::ProfileListResponse { profiles, current_profile }
		}
		"GetCurrentProfile" => {
			let profile_name = response_data.get("profileName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::CurrentProfileResponse { profile_name }
		}
		"GetSceneCollectionList" => {
			let collections = response_data
				.get("sceneCollections")
				.and_then(Value::as_array)
				.map(|arr| arr.iter().filter_map(|c| c.as_str().map(String::from)).collect())
				.unwrap_or_default();
			let current_collection = response_data.get("currentSceneCollectionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::SceneCollectionListResponse { collections, current_collection }
		}
		"GetCurrentSceneCollection" => {
			let collection_name = response_data.get("sceneCollectionName").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::CurrentCollectionResponse { collection_name }
		}
		"GetVirtualCamStatus" => {
			let active = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::VirtualCamStatusResponse { active }
		}
		"GetReplayBufferStatus" => {
			let active = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::ReplayBufferStatusResponse { active }
		}
		"GetStudioModeEnabled" => {
			let enabled = response_data.get("studioModeEnabled").and_then(Value::as_bool).unwrap_or(false);
			ObsEvent::StudioModeResponse { enabled }
		}
		"GetStats" => {
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
			ObsEvent::StatsResponse { stats }
		}
		"GetCurrentSceneTransition" => {
			let transition_name = response_data.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
			let transition_duration = response_data.get("transitionDuration").and_then(Value::as_u64).unwrap_or(0) as u32;
			ObsEvent::CurrentTransitionResponse {
				transition_name,
				transition_duration,
			}
		}
		"GetSceneTransitionList" => {
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
			ObsEvent::TransitionListResponse { transitions }
		}
		"GetSourceFilterList" => {
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
			ObsEvent::FilterListResponse { source_name, filters }
		}
		"GetHotkeyList" => {
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
			ObsEvent::HotkeyListResponse { hotkeys }
		}
		"GetVersion" => {
			let obs_version = response_data.get("obsVersion").and_then(Value::as_str).unwrap_or("").to_string();
			let websocket_version = response_data.get("obsWebSocketVersion").and_then(Value::as_str).unwrap_or("").to_string();
			ObsEvent::VersionResponse { obs_version, websocket_version }
		}
		_ => {
			debug!("Unhandled response type: {}", request_type);
			ObsEvent::UnknownResponse {
				request_type: request_type.to_string(),
				data: response_data.clone(),
			}
		}
	};

	Ok(event)
}

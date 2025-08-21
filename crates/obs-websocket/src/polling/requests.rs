use super::*;

/// Polling frequency levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollingFrequency {
	High,   // Every second
	Medium, // Every 5 seconds
	Low,    // Every 30 seconds
}

/// All available OBS request types with their parameters
#[derive(Debug, Clone)]
pub enum ObsRequestType {
	// Stream and Recording
	StreamStatus,
	StartStream,
	StopStream,
	RecordStatus,
	StartRecord,
	StopRecord,

	// Scene Management
	SceneList,
	CurrentScene,
	SetCurrentProgramScene(String),

	// Source Management
	SourcesList,
	InputsList,

	// Audio Management
	InputMute(String),
	SetInputMute(String, String),
	InputVolume(String),
	SetInputVolume(String, String),

	// Profile and Collection
	ProfileList,
	CurrentProfile,
	SceneCollectionList,
	CurrentSceneCollection,

	// Virtual Camera
	VirtualCamStatus,
	ToggleVirtualCam,

	// Replay Buffer
	ReplayBufferStatus,
	ToggleReplayBuffer,

	// Studio Mode
	StudioModeStatus,
	ToggleStudioMode,

	// Stats
	Stats,

	// Transitions
	CurrentTransition,
	TransitionList,

	// Hotkeys
	HotkeyList,

	// Version
	Version,
}

impl ObsRequestType {
	pub fn to_polling_request(&self) -> PollingRequest {
		match self {
			Self::StreamStatus => PollingRequest::new("GetStreamStatus", "stream"),
			Self::StartStream => PollingRequest::new("StartStream", "start_stream"),
			Self::StopStream => PollingRequest::new("StopStream", "stop_stream"),
			Self::RecordStatus => PollingRequest::new("GetRecordStatus", "record"),
			Self::StartRecord => PollingRequest::new("StartRecord", "start_record"),
			Self::StopRecord => PollingRequest::new("StopRecord", "stop_record"),
			Self::SceneList => PollingRequest::new("GetSceneList", "scenes"),
			Self::CurrentScene => PollingRequest::new("GetCurrentProgramScene", "current_scene"),
			Self::SetCurrentProgramScene(scene_name) => PollingRequest::new("SetCurrentProgramScene", "set_scene").with_data(json!({"sceneName": scene_name})),
			Self::SourcesList => PollingRequest::new("GetSourcesList", "sources"),
			Self::InputsList => PollingRequest::new("GetInputList", "inputs"),
			Self::InputMute(input_name) => PollingRequest::new("GetInputMute", "audio_mute").with_data(json!({ "inputName": input_name })),
			Self::SetInputMute(input_name, muted) => PollingRequest::new("SetInputMute", "set_mute").with_data(json!({ "inputName": input_name, "inputMuted": muted  })),
			Self::InputVolume(input_name) => PollingRequest::new("GetInputVolume", "audio_volume").with_data(json!({ "inputName": input_name })),
			Self::SetInputVolume(input_name, volume) => PollingRequest::new("SetInputVolume", "set_volume").with_data(json!({ "inputName": input_name, "inputVolume": volume  })),
			Self::ProfileList => PollingRequest::new("GetProfileList", "profiles"),
			Self::CurrentProfile => PollingRequest::new("GetCurrentProfile", "current_profile"),
			Self::SceneCollectionList => PollingRequest::new("GetSceneCollectionList", "collections"),
			Self::CurrentSceneCollection => PollingRequest::new("GetCurrentSceneCollection", "current_collection"),
			Self::VirtualCamStatus => PollingRequest::new("GetVirtualCamStatus", "virtual_cam"),
			Self::ToggleVirtualCam => PollingRequest::new("ToggleVirtualCam", "toggle_vcam"),
			Self::ReplayBufferStatus => PollingRequest::new("GetReplayBufferStatus", "replay_buffer"),
			Self::ToggleReplayBuffer => PollingRequest::new("ToggleReplayBuffer", "toggle_replay"),
			Self::StudioModeStatus => PollingRequest::new("GetStudioModeEnabled", "studio_mode"),
			Self::ToggleStudioMode => PollingRequest::new("ToggleStudioMode", "toggle_studio"),
			Self::Stats => PollingRequest::new("GetStats", "stats"),
			Self::CurrentTransition => PollingRequest::new("GetCurrentSceneTransition", "current_transition"),
			Self::TransitionList => PollingRequest::new("GetSceneTransitionList", "transitions"),
			Self::HotkeyList => PollingRequest::new("GetHotkeyList", "hotkeys"),
			Self::Version => PollingRequest::new("GetVersion", "version"),
		}
	}
}

#[derive(Debug, Clone)]
pub struct PollingRequest {
	pub request_type: String,
	pub prefix: String,
	pub request_data: Option<serde_json::Value>,
}

impl PollingRequest {
	pub fn new(request_type: &str, prefix: &str) -> Self {
		PollingRequest {
			request_type: request_type.to_string(),
			prefix: prefix.to_string(),
			request_data: None,
		}
	}

	pub fn with_data(mut self, data: serde_json::Value) -> Self {
		self.request_data = Some(data);
		self
	}
}

/// Request ID generation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestIdStrategy {
	/// Use UUID v4 for maximum uniqueness (recommended)
	Uuid,
	// /// Use timestamp + random suffix for human-readable IDs
	// Timestamp,
	// /// Use atomic counter (legacy, not recommended)
	// Sequential,
}

/// Comprehensive OBS polling requests
#[derive(Debug, Clone)]
pub struct ObsPollingRequests {
	id_strategy: RequestIdStrategy,
	// Only used for Sequential strategy - avoid if possible
	// request_counter: Option<AtomicU32>,
}

impl ObsPollingRequests {
	/// Create with UUID strategy (recommended)
	pub const fn new() -> Self {
		Self::with_strategy(RequestIdStrategy::Uuid)
	}

	/// Create with specific ID generation strategy
	pub const fn with_strategy(strategy: RequestIdStrategy) -> Self {
		// let request_counter = match strategy {
		// RequestIdStrategy::Sequential => Some(AtomicU32::new(0)),
		// _ => None,
		// };

		Self { id_strategy: strategy }
	}

	/// Generate a unique request ID - thread-safe, no mut required
	fn generate_id(&self, prefix: &str) -> String {
		match self.id_strategy {
			RequestIdStrategy::Uuid => {
				// Use UUID v4 for maximum uniqueness
				format!("{}-{}", prefix, uuid::Uuid::new_v4().simple())
			} // RequestIdStrategy::Timestamp => {
			  // 	// Use timestamp + random 4-digit suffix
			  // 	let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
			  // 	let random_suffix = fastrand::u16(1000..=9999);
			  // 	format!("{}-{}-{}", prefix, timestamp, random_suffix)
			  // }
			  // RequestIdStrategy::Sequential => {
			  // 	// Legacy atomic counter approach
			  // 	if let Some(ref counter) = self.request_counter {
			  // 		let id = counter.fetch_add(1, Ordering::Relaxed);
			  // 		format!("{}-{}", prefix, id)
			  // 	} else {
			  // 		// Fallback to UUID if counter is somehow missing
			  // 		format!("{}-{}", prefix, uuid::Uuid::new_v4().simple())
			  // 	}
			  // }
		}
	}

	/// Generate requests from a list of PollingRequest configs
	/// No longer requires &mut self!
	pub fn generate_requests(&self, requests: &[PollingRequest]) -> Vec<serde_json::Value> {
		requests.iter().map(|req| self.create_request(req)).collect()
	}

	pub fn create_request(&self, request: &PollingRequest) -> serde_json::Value {
		let request_id = self.generate_id(&request.prefix);
		let mut json_req = json!({
			"op": 6,
			"d": {
				"requestType": request.request_type,
				"requestId": request_id
			}
		});

		if let Some(data) = &request.request_data {
			json_req["d"]["requestData"] = data.clone();
		}

		json_req
	}
}

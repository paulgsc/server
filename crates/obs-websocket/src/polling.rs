use futures_util::sink::SinkExt;
use futures_util::stream::SplitSink;
use serde_json::json;
use std::time::Duration;
use tokio::time::interval;
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, WebSocketStream};
use tracing::{error, info};

pub type WsSink = SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, TungsteniteMessage>;

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
	fn to_polling_request(&self) -> PollingRequest {
		match self {
			Self::StreamStatus => PollingRequest::new("GetStreamStatus", "stream"),
			Self::StartStream => PollingRequest::new("StartStream", "start_stream"),
			Self::StopStream => PollingRequest::new("StopStream", "stop_stream"),
			Self::RecordStatus => PollingRequest::new("GetRecordStatus", "record"),
			Self::StartRecord => PollingRequest::new("StartRecord", "start_record"),
			Self::StopRecord => PollingRequest::new("StopRecord", "stop_record"),
			Self::SceneList => PollingRequest::new("GetSceneList", "scenes"),
			Self::CurrentScene => PollingRequest::new("GetCurrentProgramScene", "current_scene"),
			Self::SetCurrentProgramScene(scene_name) => PollingRequest::new("GetCurrentProgramScene", "current_scene").with_data(json!({"sceneName": scene_name})),
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

/// Configuration automatically built from frequency-tagged requests
#[derive(Debug, Clone)]
pub struct PollingConfig {
	pub high_frequency_requests: Vec<PollingRequest>,
	pub medium_frequency_requests: Vec<PollingRequest>,
	pub low_frequency_requests: Vec<PollingRequest>,
}

impl PollingConfig {
	/// Create configuration from slice of (RequestType, Frequency) tuples
	pub fn from_request_slice(requests: &[(ObsRequestType, PollingFrequency)]) -> Self {
		let mut config = Self {
			high_frequency_requests: Vec::new(),
			medium_frequency_requests: Vec::new(),
			low_frequency_requests: Vec::new(),
		};

		for (request_type, frequency) in requests {
			let polling_request = request_type.to_polling_request();
			match frequency {
				PollingFrequency::High => config.high_frequency_requests.push(polling_request),
				PollingFrequency::Medium => config.medium_frequency_requests.push(polling_request),
				PollingFrequency::Low => config.low_frequency_requests.push(polling_request),
			}
		}

		config
	}
}

/// Comprehensive OBS polling requests
#[derive(Debug, Clone)]
pub struct ObsPollingRequests {
	request_id: u32,
}

impl ObsPollingRequests {
	pub const fn new() -> Self {
		Self { request_id: 0 }
	}

	pub fn next_id(&mut self) -> u32 {
		self.request_id += 1;
		self.request_id
	}

	/// Generate requests from a list of PollingRequest configs
	pub fn generate_requests(&mut self, requests: &[PollingRequest]) -> Vec<serde_json::Value> {
		requests.iter().map(|req| self.create_request(req)).collect()
	}

	fn create_request(&mut self, request: &PollingRequest) -> serde_json::Value {
		let id = self.next_id();
		let mut json_req = json!({
			"op": 6,
			"d": {
				"requestType": request.request_type,
				"requestId": format!("{}-{}", request.prefix, id)
			}
		});

		if let Some(data) = &request.request_data {
			json_req["d"]["requestData"] = data.clone();
		}

		json_req
	}
}

/// Configurable polling manager
pub struct ObsPollingManager {
	requests: ObsPollingRequests,
	config: PollingConfig,
	high_freq_interval: Duration,
	medium_freq_interval: Duration,
	low_freq_interval: Duration,
}

impl ObsPollingManager {
	pub fn new(config: PollingConfig) -> Self {
		Self {
			requests: ObsPollingRequests::new(),
			config,
			high_freq_interval: Duration::from_secs(1),   // Every second
			medium_freq_interval: Duration::from_secs(5), // Every 5 seconds
			low_freq_interval: Duration::from_secs(30),   // Every 30 seconds
		}
	}

	/// Create from a slice of (RequestType, Frequency) tuples
	pub fn from_request_slice(requests: &[(ObsRequestType, PollingFrequency)]) -> Self {
		Self::new(PollingConfig::from_request_slice(requests))
	}

	/// Main polling loop with configurable requests
	pub async fn start_polling_loop(mut self, mut sink: WsSink, mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<crate::OBSCommand>) {
		let mut high_freq_timer = interval(self.high_freq_interval);
		let mut medium_freq_timer = interval(self.medium_freq_interval);
		let mut low_freq_timer = interval(self.low_freq_interval);

		// Skip the first tick to avoid immediate execution
		high_freq_timer.tick().await;
		medium_freq_timer.tick().await;
		low_freq_timer.tick().await;

		loop {
			tokio::select! {
				// High frequency polling (1 second)
				_ = high_freq_timer.tick() => {
					if !self.config.high_frequency_requests.is_empty() {
						let requests = self.requests.generate_requests(&self.config.high_frequency_requests);
						if let Err(e) = self.send_requests(&mut sink, requests).await {
							error!("Failed to send high frequency requests: {}", e);
							return;
						}
					}
				}

				// Medium frequency polling (5 seconds)
				_ = medium_freq_timer.tick() => {
					if !self.config.medium_frequency_requests.is_empty() {
						let requests = self.requests.generate_requests(&self.config.medium_frequency_requests);
						if let Err(e) = self.send_requests(&mut sink, requests).await {
							error!("Failed to send medium frequency requests: {}", e);
							return;
						}
					}
				}

				// Low frequency polling (30 seconds)
				_ = low_freq_timer.tick() => {
					if !self.config.low_frequency_requests.is_empty() {
						let requests = self.requests.generate_requests(&self.config.low_frequency_requests);
						if let Err(e) = self.send_requests(&mut sink, requests).await {
							error!("Failed to send low frequency requests: {}", e);
							return;
						}
					}
				}

				// Handle manual commands
				Some(cmd) = cmd_rx.recv() => {
					match cmd {
						crate::OBSCommand::SendRequest(req) => {
							if let Err(e) = sink.send(TungsteniteMessage::Text(req.to_string().into())).await {
								error!("Failed to send manual request: {}", e);
								return;
							}
							if let Err(e) = sink.flush().await {
								error!("Failed to flush sink: {}", e);
								return;
							}
						}
						crate::OBSCommand::Disconnect => {
							info!("Received disconnect command");
							return;
						}
					}
				}
			}
		}
	}

	/// Send a batch of requests to OBS
	async fn send_requests(&mut self, sink: &mut WsSink, requests: Vec<serde_json::Value>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		// TODO: Let's not heap allocate stuff?
		let mut send_errors = Vec::new();

		for req in requests {
			if let Err(e) = sink.send(TungsteniteMessage::Text(req.to_string().into())).await {
				send_errors.push(e);
			}
		}
		sink.flush().await?;

		if !send_errors.is_empty() {
			return Err(send_errors.into_iter().next().unwrap().into());
		}
		Ok(())
	}
}

/// Utility functions for specific OBS operations
pub struct ObsRequestBuilder {
	requests: ObsPollingRequests,
}

impl ObsRequestBuilder {
	pub fn new() -> Self {
		Self {
			requests: ObsPollingRequests::new(),
		}
	}

	/// Start streaming
	pub fn start_stream(&mut self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::StartStream.to_polling_request())
	}

	/// Stop streaming
	pub fn stop_stream(&mut self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::StopStream.to_polling_request())
	}

	/// Start recording
	pub fn start_recording(&mut self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::StartRecord.to_polling_request())
	}

	/// Stop recording
	pub fn stop_recording(&mut self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::StopRecord.to_polling_request())
	}

	/// Switch to a specific scene
	pub fn switch_scene(&mut self, scene_name: &str) -> serde_json::Value {
		self
			.requests
			.create_request(&ObsRequestType::SetCurrentProgramScene(scene_name.to_string()).to_polling_request())
	}

	// /// Set source visibility
	// pub fn set_source_visibility(&mut self, scene_name: &str, source_name: &str, visible: bool) -> serde_json::Value {
	// 	self.requests.create_request_with_data(
	// 		"SetSceneItemEnabled",
	// 		"set_visibility",
	// 		json!({
	// 			"sceneName": scene_name,
	// 			"sceneItemId": source_name,
	// 			"sceneItemEnabled": visible
	// 		}),
	// 	)
	// }

	/// Mute/unmute audio source
	pub fn set_input_mute(&mut self, input_name: &str, muted: bool) -> serde_json::Value {
		self
			.requests
			.create_request(&ObsRequestType::SetInputMute(input_name.to_string(), muted.to_string()).to_polling_request())
	}

	/// Set audio volume
	pub fn set_input_volume(&mut self, input_name: &str, volume: f64) -> serde_json::Value {
		self
			.requests
			.create_request(&ObsRequestType::SetInputVolume(input_name.to_string(), volume.to_string()).to_polling_request())
	}

	/// Toggle studio mode
	pub fn toggle_studio_mode(&mut self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::ToggleStudioMode.to_polling_request())
	}

	/// Start/stop virtual camera
	pub fn toggle_virtual_camera(&mut self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::ToggleVirtualCam.to_polling_request())
	}

	/// Start/stop replay buffer
	pub fn toggle_replay_buffer(&mut self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::ToggleReplayBuffer.to_polling_request())
	}
}

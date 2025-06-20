use futures_util::sink::SinkExt;
use futures_util::stream::SplitSink;
use serde_json::json;
use std::time::Duration;
use tokio::time::interval;
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, WebSocketStream};
use tracing::{error, info};

pub type WsSink = SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, TungsteniteMessage>;

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

	// /// Generate all polling requests for comprehensive OBS monitoring
	// pub fn generate_polling_requests(&mut self) -> Vec<serde_json::Value> {
	// 	vec![
	// 		// Stream and Recording Status
	// 		self.create_request("GetStreamStatus", "stream"),
	// 		self.create_request("GetRecordStatus", "record"),
	// 		// Scene Management
	// 		self.create_request("GetSceneList", "scenes"),
	// 		self.create_request("GetCurrentProgramScene", "current_scene"),
	// 		// Source Management
	// 		self.create_request("GetSourcesList", "sources"),
	// 		self.create_request("GetInputList", "inputs"),
	// 		// Audio Management
	// 		self.create_request("GetInputMute", "audio_mute"),
	// 		self.create_request("GetInputVolume", "audio_volume"),
	// 		// Profile and Collection
	// 		self.create_request("GetProfileList", "profiles"),
	// 		self.create_request("GetCurrentProfile", "current_profile"),
	// 		self.create_request("GetSceneCollectionList", "collections"),
	// 		self.create_request("GetCurrentSceneCollection", "current_collection"),
	// 		// Virtual Camera
	// 		self.create_request("GetVirtualCamStatus", "virtual_cam"),
	// 		// Replay Buffer
	// 		self.create_request("GetReplayBufferStatus", "replay_buffer"),
	// 		// Studio Mode
	// 		self.create_request("GetStudioModeEnabled", "studio_mode"),
	// 		// Stats
	// 		self.create_request("GetStats", "stats"),
	// 		// Transitions
	// 		self.create_request("GetCurrentSceneTransition", "current_transition"),
	// 		self.create_request("GetSceneTransitionList", "transitions"),
	// 		// Filters (we'll get these per source later)
	// 		// self.create_request("GetSourceFilterList", "filters"),

	// 		// Hotkeys
	// 		self.create_request("GetHotkeyList", "hotkeys"),
	// 		// Version info (less frequent)
	// 		self.create_request("GetVersion", "version"),
	// 	]
	// }

	/// Generate high-frequency requests (every second)
	pub fn generate_high_frequency_requests(&mut self) -> Vec<serde_json::Value> {
		vec![
			self.create_request("GetStreamStatus", "stream"),
			self.create_request("GetRecordStatus", "record"),
			self.create_request("GetCurrentProgramScene", "current_scene"),
			self.create_request("GetStats", "stats"),
		]
	}

	/// Generate medium-frequency requests (every 5 seconds)
	pub fn generate_medium_frequency_requests(&mut self) -> Vec<serde_json::Value> {
		vec![
			self.create_request("GetSceneList", "scenes"),
			self.create_request("GetSourcesList", "sources"),
			self.create_request("GetInputList", "inputs"),
			self.create_request("GetVirtualCamStatus", "virtual_cam"),
			self.create_request("GetReplayBufferStatus", "replay_buffer"),
			self.create_request("GetStudioModeEnabled", "studio_mode"),
		]
	}

	/// Generate low-frequency requests (every 30 seconds)
	pub fn generate_low_frequency_requests(&mut self) -> Vec<serde_json::Value> {
		vec![
			self.create_request("GetProfileList", "profiles"),
			self.create_request("GetCurrentProfile", "current_profile"),
			self.create_request("GetSceneCollectionList", "collections"),
			self.create_request("GetCurrentSceneCollection", "current_collection"),
			self.create_request("GetCurrentSceneTransition", "current_transition"),
			self.create_request("GetSceneTransitionList", "transitions"),
			self.create_request("GetHotkeyList", "hotkeys"),
			self.create_request("GetVersion", "version"),
		]
	}

	fn create_request(&mut self, request_type: &str, prefix: &str) -> serde_json::Value {
		let id = self.next_id();
		json!({
			"op": 6,
			"d": {
				"requestType": request_type,
				"requestId": format!("{}-{}", prefix, id)
			}
		})
	}

	/// Create a request with specific data
	pub fn create_request_with_data(&mut self, request_type: &str, prefix: &str, data: serde_json::Value) -> serde_json::Value {
		let id = self.next_id();
		json!({
			"op": 6,
			"d": {
				"requestType": request_type,
				"requestId": format!("{}-{}", prefix, id),
				"requestData": data
			}
		})
	}
}

/// Enhanced polling manager with multiple frequency tiers
pub struct ObsPollingManager {
	requests: ObsPollingRequests,
	high_freq_interval: Duration,
	medium_freq_interval: Duration,
	low_freq_interval: Duration,
}

impl ObsPollingManager {
	pub fn new() -> Self {
		Self {
			requests: ObsPollingRequests::new(),
			high_freq_interval: Duration::from_secs(1),   // Every second
			medium_freq_interval: Duration::from_secs(5), // Every 5 seconds
			low_freq_interval: Duration::from_secs(30),   // Every 30 seconds
		}
	}

	/// Main polling loop with multiple frequency tiers
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
					let requests = self.requests.generate_high_frequency_requests();
					if let Err(e) = self.send_requests(&mut sink, requests).await {
						error!("Failed to send high frequency requests: {}", e);
						return;
					}
				}

				// Medium frequency polling (5 seconds)
				_ = medium_freq_timer.tick() => {
					let requests = self.requests.generate_medium_frequency_requests();
					if let Err(e) = self.send_requests(&mut sink, requests).await {
						error!("Failed to send medium frequency requests: {}", e);
						return;
					}
				}

				// Low frequency polling (30 seconds)
				_ = low_freq_timer.tick() => {
					let requests = self.requests.generate_low_frequency_requests();
					if let Err(e) = self.send_requests(&mut sink, requests).await {
						error!("Failed to send low frequency requests: {}", e);
						return;
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
		for req in requests {
			sink.send(TungsteniteMessage::Text(req.to_string().into())).await?;
		}
		sink.flush().await?;
		Ok(())
	}

	// /// Send a single request immediately
	// pub async fn send_immediate_request(
	// 	&mut self,
	// 	sink: &mut WsSink,
	// 	request_type: &str,
	// 	prefix: &str,
	// 	data: Option<serde_json::Value>,
	// ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
	// 	let request = if let Some(data) = data {
	// 		self.requests.create_request_with_data(request_type, prefix, data)
	// 	} else {
	// 		self.requests.create_request(request_type, prefix)
	// 	};

	// 	sink.send(TungsteniteMessage::Text(request.to_string().into())).await?;
	// 	sink.flush().await?;
	// 	Ok(())
	// }

	// /// Customize polling intervals
	// pub fn set_intervals(&mut self, high: Duration, medium: Duration, low: Duration) {
	// 	self.high_freq_interval = high;
	// 	self.medium_freq_interval = medium;
	// 	self.low_freq_interval = low;
	// }
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
		self.requests.create_request("StartStream", "start_stream")
	}

	/// Stop streaming
	pub fn stop_stream(&mut self) -> serde_json::Value {
		self.requests.create_request("StopStream", "stop_stream")
	}

	/// Start recording
	pub fn start_recording(&mut self) -> serde_json::Value {
		self.requests.create_request("StartRecord", "start_record")
	}

	/// Stop recording
	pub fn stop_recording(&mut self) -> serde_json::Value {
		self.requests.create_request("StopRecord", "stop_record")
	}

	/// Switch to a specific scene
	pub fn switch_scene(&mut self, scene_name: &str) -> serde_json::Value {
		self
			.requests
			.create_request_with_data("SetCurrentProgramScene", "switch_scene", json!({ "sceneName": scene_name }))
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
		self.requests.create_request_with_data(
			"SetInputMute",
			"set_mute",
			json!({
				"inputName": input_name,
				"inputMuted": muted
			}),
		)
	}

	/// Set audio volume
	pub fn set_input_volume(&mut self, input_name: &str, volume: f64) -> serde_json::Value {
		self.requests.create_request_with_data(
			"SetInputVolume",
			"set_volume",
			json!({
				"inputName": input_name,
				"inputVolumeDb": volume
			}),
		)
	}

	/// Toggle studio mode
	pub fn toggle_studio_mode(&mut self) -> serde_json::Value {
		self.requests.create_request("ToggleStudioMode", "toggle_studio")
	}

	/// Start/stop virtual camera
	pub fn toggle_virtual_camera(&mut self) -> serde_json::Value {
		self.requests.create_request("ToggleVirtualCam", "toggle_vcam")
	}

	/// Start/stop replay buffer
	pub fn toggle_replay_buffer(&mut self) -> serde_json::Value {
		self.requests.create_request("ToggleReplayBuffer", "toggle_replay")
	}
}

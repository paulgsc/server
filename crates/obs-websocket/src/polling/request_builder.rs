use super::*;

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
	pub fn start_stream(&self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::StartStream.to_polling_request())
	}

	/// Stop streaming
	pub fn stop_stream(&self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::StopStream.to_polling_request())
	}

	/// Start recording
	pub fn start_recording(&self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::StartRecord.to_polling_request())
	}

	/// Stop recording
	pub fn stop_recording(&self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::StopRecord.to_polling_request())
	}

	/// Switch to a specific scene
	pub fn switch_scene(&self, scene_name: &str) -> serde_json::Value {
		self
			.requests
			.create_request(&ObsRequestType::SetCurrentProgramScene(scene_name.to_string()).to_polling_request())
	}

	// /// Set source visibility
	// pub fn set_source_visibility(&mut self, scene_name: &str, source_name: &str, visible: bool) -> serde_json::Value {
	//	self.requests.create_request_with_data(
	//		"SetSceneItemEnabled",
	//		"set_visibility",
	//		json!({
	//			"sceneName": scene_name,
	//			"sceneItemId": source_name,
	//			"sceneItemEnabled": visible
	//		}),
	//	)
	// }

	/// Mute/unmute audio source
	pub fn set_input_mute(&self, input_name: &str, muted: bool) -> serde_json::Value {
		self
			.requests
			.create_request(&ObsRequestType::SetInputMute(input_name.to_string(), muted.to_string()).to_polling_request())
	}

	/// Set audio volume
	pub fn set_input_volume(&self, input_name: &str, volume: f64) -> serde_json::Value {
		self
			.requests
			.create_request(&ObsRequestType::SetInputVolume(input_name.to_string(), volume.to_string()).to_polling_request())
	}

	/// Toggle studio mode
	pub fn toggle_studio_mode(&self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::ToggleStudioMode.to_polling_request())
	}

	/// Start/stop virtual camera
	pub fn toggle_virtual_camera(&self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::ToggleVirtualCam.to_polling_request())
	}

	/// Start/stop replay buffer
	pub fn toggle_replay_buffer(&self) -> serde_json::Value {
		self.requests.create_request(&ObsRequestType::ToggleReplayBuffer.to_polling_request())
	}
}

use crate::polling::PollingError;
use crate::types::*;
use serde::Serialize;
use uuid::Uuid;

type Result<T> = std::result::Result<T, PollingError>;

/// Utility functions for specific OBS operations
pub struct ObsRequestBuilder;

impl ObsRequestBuilder {
	/// Generate a unique request ID
	fn generate_id(prefix: &str) -> String {
		format!("{}-{}", prefix, Uuid::new_v4().simple())
	}

	/// Create a generic OBS request using the new type system
	pub fn create_request<T>(request_type: ObsRequestType, params: Option<T>) -> Result<serde_json::Value>
	where
		T: Serialize,
	{
		let request_id = Self::generate_id("req");

		let val = serde_json::to_value(ObsRequest {
			op_code: 6,
			d: RequestData {
				t: request_type,
				id: request_id,
				p: params,
			},
		})?;

		Ok(val)
	}

	/// Start streaming
	pub fn start_stream() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::StartStream, None::<()>)
	}

	/// Stop streaming  
	pub fn stop_stream() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::StopStream, None::<()>)
	}

	/// Start recording
	pub fn start_recording() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::StartRecord, None::<()>)
	}

	/// Stop recording
	pub fn stop_recording() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::StopRecord, None::<()>)
	}

	/// Switch to a specific scene
	pub fn switch_scene(scene_name: &str) -> Result<serde_json::Value> {
		Self::create_request(
			ObsRequestType::SetCurrentProgramScene,
			Some(SetCurrentProgramSceneParams {
				scene_name: scene_name.to_string(),
			}),
		)
	}

	/// Mute/unmute audio source
	pub fn set_input_mute(input_name: &str, muted: bool) -> Result<serde_json::Value> {
		Self::create_request(
			ObsRequestType::SetInputMute,
			Some(SetInputMuteParams {
				n: input_name.to_string(),
				b: muted,
			}),
		)
	}

	/// Set audio volume
	pub fn set_input_volume(input_name: &str, volume: f64) -> Result<serde_json::Value> {
		Self::create_request(
			ObsRequestType::SetInputVolume,
			Some(SetInputVolumeParams {
				n: input_name.to_string(),
				v: volume,
			}),
		)
	}

	/// Toggle studio mode
	pub fn toggle_studio_mode(enabled: bool) -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::SetStudioModeEnabled, Some(SetStudioModeEnabledParams { b: enabled }))
	}

	/// Start virtual camera
	pub fn start_virtual_camera() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::StartVirtualCam, None::<()>)
	}

	/// Stop virtual camera
	pub fn stop_virtual_camera() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::StopVirtualCam, None::<()>)
	}

	/// Start replay buffer
	pub fn start_replay_buffer() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::StartReplayBuffer, None::<()>)
	}

	/// Stop replay buffer
	pub fn stop_replay_buffer() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::StopReplayBuffer, None::<()>)
	}

	/// Get stream status
	pub fn get_stream_status() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::GetStreamStatus, None::<()>)
	}

	/// Get recording status  
	pub fn get_record_status() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::GetRecordStatus, None::<()>)
	}

	/// Get scene list
	pub fn get_scene_list() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::GetSceneList, None::<()>)
	}

	/// Get current scene
	pub fn get_current_scene() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::GetCurrentProgramScene, None::<()>)
	}

	/// Get input mute status
	pub fn get_input_mute(input_name: &str) -> Result<serde_json::Value> {
		Self::create_request(
			ObsRequestType::GetInputMute,
			Some(SetInputMuteParams {
				n: input_name.to_string(),
				b: false, // This param isn't used for GET requests but required by struct
			}),
		)
	}

	/// Get input volume
	pub fn get_input_volume(input_name: &str) -> Result<serde_json::Value> {
		Self::create_request(
			ObsRequestType::GetInputVolume,
			Some(SetInputVolumeParams {
				n: input_name.to_string(),
				v: 0.0, // This param isn't used for GET requests but required by struct
			}),
		)
	}

	/// Set YouTube stream settings
	pub fn set_youtube_stream(
		stream_key: &str,
		title: &str,
		description: &str,
		category: &str,
		privacy: YouTubePrivacy,
		unlisted: bool,
		tags: Vec<String>,
	) -> Result<serde_json::Value> {
		Self::create_request(
			ObsRequestType::SetStreamServiceSettings,
			Some(SetStreamServiceSettingsParams {
				service_type: "rtmp_custom".to_string(), // or "youtube_live" if using YouTube service
				settings: StreamServiceSettings {
					stream_key: Some(stream_key.to_string()),
					server: Some("rtmp://a.rtmp.youtube.com/live2".to_string()),
					title: Some(title.to_string()),
					description: Some(description.to_string()),
					game: Some(category.to_string()),
					privacy: Some(privacy.as_str().to_string()),
					unlisted: Some(unlisted),
					tags: Some(tags),
				},
			}),
		)
	}

	/// Get current stream service settings
	pub fn get_stream_service_settings() -> Result<serde_json::Value> {
		Self::create_request(ObsRequestType::GetStreamServiceSettings, None::<()>)
	}
}

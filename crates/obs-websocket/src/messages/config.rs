use crate::messages::ObsRequestType;

/// Configuration for OBS WebSocket initialization
#[derive(Debug, Clone)]
pub struct InitializationConfig {
	pub requests: Vec<(ObsRequestType, &'static str)>,
}

impl Default for InitializationConfig {
	fn default() -> Self {
		Self {
			requests: vec![
				(ObsRequestType::GetSceneList, "scenes-init"),
				(ObsRequestType::GetStreamStatus, "stream-init"),
				(ObsRequestType::GetRecordStatus, "recording-init"),
				(ObsRequestType::GetCurrentProgramScene, "current-scene-init"),
				(ObsRequestType::GetVirtualCamStatus, "vcam-init"),
				(ObsRequestType::GetStudioModeEnabled, "studio-init"),
			],
		}
	}
}

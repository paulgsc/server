use super::*;

/// Configuration automatically built from frequency-tagged requests
#[derive(Debug, Clone)]
pub struct PollingConfig {
	pub high_frequency_requests: Vec<PollingRequest>,
	pub medium_frequency_requests: Vec<PollingRequest>,
	pub low_frequency_requests: Vec<PollingRequest>,
}

impl Default for PollingConfig {
	/// Returns a default polling configuration suitable for basic monitoring.
	fn default() -> Self {
		Self::default_monitoring()
	}
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

	/// Utility functions for creating default polling configurations
	/// Create a default configuration for basic OBS monitoring
	pub fn default_monitoring() -> Self {
		Self::from_request_slice(&[
			(ObsRequestType::StreamStatus, PollingFrequency::High),
			(ObsRequestType::RecordStatus, PollingFrequency::High),
			(ObsRequestType::CurrentScene, PollingFrequency::Medium),
			(ObsRequestType::VirtualCamStatus, PollingFrequency::Medium),
			(ObsRequestType::StudioModeStatus, PollingFrequency::Medium),
			(ObsRequestType::Stats, PollingFrequency::Low),
			(ObsRequestType::SceneList, PollingFrequency::Low),
			(ObsRequestType::InputsList, PollingFrequency::Low),
		])
	}

	/// Create a lightweight configuration for minimal polling
	pub fn minimal_monitoring() -> Self {
		Self::from_request_slice(&[
			(ObsRequestType::StreamStatus, PollingFrequency::Medium),
			(ObsRequestType::RecordStatus, PollingFrequency::Medium),
			(ObsRequestType::CurrentScene, PollingFrequency::Low),
		])
	}

	/// Create a comprehensive configuration for full monitoring
	pub fn comprehensive_monitoring() -> Self {
		Self::from_request_slice(&[
			// High frequency - critical status updates
			(ObsRequestType::StreamStatus, PollingFrequency::High),
			(ObsRequestType::RecordStatus, PollingFrequency::High),
			(ObsRequestType::CurrentScene, PollingFrequency::High),
			// Medium frequency - important but not critical
			(ObsRequestType::VirtualCamStatus, PollingFrequency::Medium),
			(ObsRequestType::ReplayBufferStatus, PollingFrequency::Medium),
			(ObsRequestType::StudioModeStatus, PollingFrequency::Medium),
			(ObsRequestType::CurrentTransition, PollingFrequency::Medium),
			// Low frequency - configuration and setup info
			(ObsRequestType::Stats, PollingFrequency::Low),
			(ObsRequestType::SceneList, PollingFrequency::Low),
			(ObsRequestType::InputsList, PollingFrequency::Low),
			(ObsRequestType::ProfileList, PollingFrequency::Low),
			(ObsRequestType::CurrentProfile, PollingFrequency::Low),
			(ObsRequestType::SceneCollectionList, PollingFrequency::Low),
			(ObsRequestType::CurrentSceneCollection, PollingFrequency::Low),
			(ObsRequestType::TransitionList, PollingFrequency::Low),
			(ObsRequestType::Version, PollingFrequency::Low),
		])
	}
}

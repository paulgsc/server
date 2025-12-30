use crate::polling::{ObsRequestBuilder, PollingError};
use crate::types::ObsRequestType;

type Result<T> = std::result::Result<Option<T>, PollingError>;
type VResult<T> = std::result::Result<Vec<T>, PollingError>;
type FResult = std::result::Result<Vec<serde_json::Value>, PollingError>;

/// Polling frequency levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollingFrequency {
	High,   // Every second
	Medium, // Every 5 seconds
	Low,    // Every 30 seconds
}

/// Configuration automatically built from frequency-tagged requests
#[derive(Debug, Clone)]
pub struct PollingConfig {
	pub high_frequency_requests: Vec<ObsRequestType>,
	pub medium_frequency_requests: Vec<ObsRequestType>,
	pub low_frequency_requests: Vec<ObsRequestType>,
}

impl Default for PollingConfig {
	/// Returns a default polling configuration suitable for basic monitoring.
	fn default() -> Self {
		Self::default_monitoring()
	}
}

impl From<&[(ObsRequestType, PollingFrequency)]> for PollingConfig {
	fn from(requests: &[(ObsRequestType, PollingFrequency)]) -> Self {
		let mut config = Self {
			high_frequency_requests: Vec::new(),
			medium_frequency_requests: Vec::new(),
			low_frequency_requests: Vec::new(),
		};
		for (request_type, frequency) in requests {
			match frequency {
				PollingFrequency::High => config.high_frequency_requests.push(request_type.clone()),
				PollingFrequency::Medium => config.medium_frequency_requests.push(request_type.clone()),
				PollingFrequency::Low => config.low_frequency_requests.push(request_type.clone()),
			}
		}
		config
	}
}

impl From<Vec<(ObsRequestType, PollingFrequency)>> for PollingConfig {
	fn from(requests: Vec<(ObsRequestType, PollingFrequency)>) -> Self {
		requests.as_slice().into()
	}
}

impl From<Box<[(ObsRequestType, PollingFrequency)]>> for PollingConfig {
	fn from(requests: Box<[(ObsRequestType, PollingFrequency)]>) -> Self {
		requests.as_ref().into()
	}
}

impl From<&PollingConfig> for Box<[(ObsRequestType, PollingFrequency)]> {
	fn from(config: &PollingConfig) -> Self {
		let mut requests = Vec::new();

		for req in &config.high_frequency_requests {
			requests.push((req.clone(), PollingFrequency::High));
		}
		for req in &config.medium_frequency_requests {
			requests.push((req.clone(), PollingFrequency::Medium));
		}
		for req in &config.low_frequency_requests {
			requests.push((req.clone(), PollingFrequency::Low));
		}
		requests.into_boxed_slice()
	}
}

impl From<PollingConfig> for Box<[(ObsRequestType, PollingFrequency)]> {
	fn from(config: PollingConfig) -> Self {
		(&config).into()
	}
}

impl PollingConfig {
	/// Generate JSON requests for a specific frequency tier
	///
	/// # Errors
	///
	/// Returns `PollingError` if request creation fails for any of the request types
	/// in the specified frequency tier.
	pub fn generate_requests_for_frequency(&self, frequency: PollingFrequency) -> FResult {
		let requests = match frequency {
			PollingFrequency::High => &self.high_frequency_requests,
			PollingFrequency::Medium => &self.medium_frequency_requests,
			PollingFrequency::Low => &self.low_frequency_requests,
		};

		Ok(
			requests
				.iter()
				.map(Self::create_request_from_type) // Fixed: removed redundant closure
				.collect::<std::result::Result<Vec<_>, _>>()? // collect results into Vec<Option<Value>>
				.into_iter()
				.flatten() // drop None, keep Some
				.collect(), // now Vec<Value>
		)
	}

	/// Create a request using the appropriate `ObsRequestBuilder` method
	/// Only handles `GET`/query requests suitable for polling
	fn create_request_from_type(request_type: &ObsRequestType) -> Result<serde_json::Value> {
		match request_type {
			// Status queries - safe for polling
			ObsRequestType::GetStreamStatus => Ok(Some(ObsRequestBuilder::get_stream_status()?)),
			ObsRequestType::GetRecordStatus => Ok(Some(ObsRequestBuilder::get_record_status()?)),
			ObsRequestType::GetCurrentProgramScene => Ok(Some(ObsRequestBuilder::get_current_scene()?)),
			ObsRequestType::GetSceneList => Ok(Some(ObsRequestBuilder::get_scene_list()?)),
			ObsRequestType::GetStreamServiceSettings => Ok(Some(ObsRequestBuilder::get_stream_service_settings()?)),

			// Configuration queries - safe for polling
			ObsRequestType::GetVirtualCamStatus
			| ObsRequestType::GetReplayBufferStatus
			| ObsRequestType::GetStudioModeEnabled
			| ObsRequestType::GetCurrentSceneTransition
			| ObsRequestType::GetStats
			| ObsRequestType::GetInputList
			| ObsRequestType::GetProfileList
			| ObsRequestType::GetCurrentProfile
			| ObsRequestType::GetSceneCollectionList
			| ObsRequestType::GetCurrentSceneCollection
			| ObsRequestType::GetSceneTransitionList
			| ObsRequestType::GetVersion => Ok(Some(ObsRequestBuilder::create_request(request_type.clone(), None::<()>)?)),

			_ => Ok(None), // Parameter-dependent queries - would need specific input names
		}
	}

	/// Generate all requests grouped by frequency
	pub fn generate_all_requests(&self) -> (VResult<serde_json::Value>, VResult<serde_json::Value>, VResult<serde_json::Value>) {
		(
			self.generate_requests_for_frequency(PollingFrequency::High),
			self.generate_requests_for_frequency(PollingFrequency::Medium),
			self.generate_requests_for_frequency(PollingFrequency::Low),
		)
	}

	/// Get all request types as a flat list with their frequencies
	#[must_use]
	pub fn get_all_request_types(&self) -> Vec<(ObsRequestType, PollingFrequency)> {
		let mut requests = Vec::new();

		for req in &self.high_frequency_requests {
			requests.push((req.clone(), PollingFrequency::High));
		}
		for req in &self.medium_frequency_requests {
			requests.push((req.clone(), PollingFrequency::Medium));
		}
		for req in &self.low_frequency_requests {
			requests.push((req.clone(), PollingFrequency::Low));
		}

		requests
	}

	/// Utility functions for creating default polling configurations
	/// Create a default configuration for basic OBS monitoring
	#[must_use]
	pub fn default_monitoring() -> Self {
		let requests: Box<[(ObsRequestType, PollingFrequency)]> = Box::new([
			// High frequency - critical status updates (all safe for polling)
			(ObsRequestType::GetStreamStatus, PollingFrequency::High),
			(ObsRequestType::GetRecordStatus, PollingFrequency::High),
			(ObsRequestType::GetCurrentProgramScene, PollingFrequency::High),
			// Medium frequency - important but not critical
			(ObsRequestType::GetStudioModeEnabled, PollingFrequency::Medium),
			(ObsRequestType::GetStats, PollingFrequency::Medium),
			// Low frequency - configuration and setup info
			(ObsRequestType::GetStudioModeEnabled, PollingFrequency::Low),
			(ObsRequestType::GetCurrentSceneTransition, PollingFrequency::Low),
			(ObsRequestType::GetSceneList, PollingFrequency::Low),
			(ObsRequestType::GetInputList, PollingFrequency::Low),
			(ObsRequestType::GetProfileList, PollingFrequency::Low),
			(ObsRequestType::GetCurrentProfile, PollingFrequency::Low),
			(ObsRequestType::GetSceneCollectionList, PollingFrequency::Low),
			(ObsRequestType::GetCurrentSceneCollection, PollingFrequency::Low),
			(ObsRequestType::GetSceneTransitionList, PollingFrequency::Low),
			(ObsRequestType::GetVersion, PollingFrequency::Low),
		]);
		Self::from(requests)
	}

	/// Create a lightweight configuration for minimal polling
	#[allow(dead_code)]
	#[must_use]
	pub fn minimal_monitoring() -> Self {
		let requests: Box<[(ObsRequestType, PollingFrequency)]> = Box::new([
			(ObsRequestType::GetStreamStatus, PollingFrequency::Medium),
			(ObsRequestType::GetRecordStatus, PollingFrequency::Medium),
			(ObsRequestType::GetCurrentProgramScene, PollingFrequency::Low),
		]);
		Self::from(requests)
	}

	/// Create a comprehensive configuration for full monitoring
	#[allow(dead_code)]
	#[must_use]
	pub fn comprehensive_monitoring() -> Self {
		let requests: Box<[(ObsRequestType, PollingFrequency)]> = Box::new([
			// High frequency - critical status updates
			(ObsRequestType::GetStreamStatus, PollingFrequency::High),
			(ObsRequestType::GetRecordStatus, PollingFrequency::High),
			(ObsRequestType::GetCurrentProgramScene, PollingFrequency::High),
			// Medium frequency - important but not critical
			(ObsRequestType::GetVirtualCamStatus, PollingFrequency::Medium),
			(ObsRequestType::GetReplayBufferStatus, PollingFrequency::Medium),
			(ObsRequestType::GetStudioModeEnabled, PollingFrequency::Medium),
			(ObsRequestType::GetCurrentSceneTransition, PollingFrequency::Medium),
			// Low frequency - configuration and setup info
			(ObsRequestType::GetStats, PollingFrequency::Low),
			(ObsRequestType::GetSceneList, PollingFrequency::Low),
			(ObsRequestType::GetInputList, PollingFrequency::Low),
			(ObsRequestType::GetProfileList, PollingFrequency::Low),
			(ObsRequestType::GetCurrentProfile, PollingFrequency::Low),
			(ObsRequestType::GetSceneCollectionList, PollingFrequency::Low),
			(ObsRequestType::GetCurrentSceneCollection, PollingFrequency::Low),
			(ObsRequestType::GetSceneTransitionList, PollingFrequency::Low),
			(ObsRequestType::GetVersion, PollingFrequency::Low),
		]);
		Self::from(requests)
	}
}

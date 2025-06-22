use obs_websocket::*;

#[tokio::main]
async fn main() {
	let config = ObsConfig::default();
	let mut obs_client = create_obs_client(config);

	let polling_requests = [
		// High frequency - every second
		(ObsRequestType::StreamStatus, PollingFrequency::High),
		(ObsRequestType::RecordStatus, PollingFrequency::High),
		(ObsRequestType::CurrentScene, PollingFrequency::High),
		(ObsRequestType::Stats, PollingFrequency::High),
		// Medium frequency - every 5 seconds
		(ObsRequestType::SceneList, PollingFrequency::Medium),
		(ObsRequestType::SourcesList, PollingFrequency::Medium),
		(ObsRequestType::InputsList, PollingFrequency::Medium),
		(ObsRequestType::VirtualCamStatus, PollingFrequency::Medium),
		(ObsRequestType::InputMute("Desktop Audio".to_string()), PollingFrequency::Medium),
		(ObsRequestType::InputVolume("Microphone".to_string()), PollingFrequency::Medium),
		// Low frequency - every 30 seconds
		(ObsRequestType::ProfileList, PollingFrequency::Low),
		(ObsRequestType::CurrentProfile, PollingFrequency::Low),
		(ObsRequestType::Version, PollingFrequency::Low),
	];

	obs_client.connect(&polling_requests).await.unwrap();

	// Simple event loop
	loop {
		println!("is connection alive: {}", obs_client.is_connected());
		match obs_client.next_event().await {
			Ok(event) => {
				// React to OBS events (launch GUI, etc.)
				println!("OBS Event: {:?}", event);
			}
			Err(e) => {
				eprintln!("Error: {}", e);
				break;
			}
		}
	}
}

use obs_websocket::*;
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();

	// Simple event loop
	bridge_obs_events().await?;

	Ok(())
}

pub fn bridge_obs_events() -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		tracing::info!("Starting OBS event bridge with internal manager");

		let obs_config = ObsConfig::default();
		let obs_manager = ObsWebSocketManager::new(obs_config, RetryConfig::default());

		loop {
			tokio::select! {
				// Handle shutdown signal
				_ = tokio::signal::ctrl_c() => {
					tracing::info!("Shutting down OBS bridge");
					let _ = obs_manager.disconnect().await;
					break;
				}

				// Main connection loop
				result = async {
					let requests = PollingConfig::default();
					let request_slice: Box<[(ObsRequestType, PollingFrequency)]> = requests.into();

					match obs_manager.connect(&request_slice).await {
						Ok(()) => {
							tracing::info!("Connected to OBS WebSocket");

							obs_manager.stream_events(|obs_event| {

								Box::pin(async move {

									tracing::debug!("new event picked up {:?}", obs_event);


								})
							}).await
						}
						Err(e) => {
							tracing::error!("Failed to connect to OBS: {}", e);
							Err(e)
						}
					}
				} => {
					if let Err(e) = result {
						tracing::error!("OBS connection error: {}", e);
					}

					// Clean disconnect before retry
					let _ = obs_manager.disconnect().await;

					// Retry delay
					tokio::time::sleep(Duration::from_secs(5)).await;
				}
			}
		}

		tracing::info!("OBS event bridge ended");
	})
}

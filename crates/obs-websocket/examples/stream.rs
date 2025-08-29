use obs_websocket::*;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

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

		// Create a cancellation token to coordinate shutdown
		let cancel_token = CancellationToken::new();
		let cancel_token_clone = cancel_token.clone();

		// Spawn a task to handle shutdown signal
		let shutdown_task = tokio::spawn(async move {
			let _ = tokio::signal::ctrl_c().await;
			tracing::info!("Shutdown signal received");
			cancel_token_clone.cancel();
		});

		loop {
			tokio::select! {
				// Check for cancellation
				_ = cancel_token.cancelled() => {
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

					// Cancellable retry delay
					tokio::select! {
						_ = cancel_token.cancelled() => {
							tracing::info!("Shutdown during retry delay");
							break;
						}
						_ = tokio::time::sleep(Duration::from_secs(5)) => {
							// Continue to next retry
						}
					}
				}
			}
		}

		// Clean up shutdown task
		shutdown_task.abort();
		tracing::info!("OBS event bridge ended");
	})
}

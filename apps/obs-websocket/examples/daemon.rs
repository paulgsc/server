use obs_websocket::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();
	let config = ObsConfig::default();
	let obs_client = create_obs_manager(config);

	let requests = PollingConfig::default();

	obs_client.connect(requests).await?;

	// Simple event loop
	loop {
		println!("is connection alive: {}", obs_client.is_healthy().await?);
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

	Ok(())
}

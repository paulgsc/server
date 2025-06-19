use obs_websocket::*;

#[tokio::main]
async fn main() {
	let config = ObsConfig::default();
	let mut obs_client = create_obs_client(config);

	obs_client.connect().await.unwrap();

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

//! Connect to OBS and log every event and status snapshot until Ctrl-C,
//! reconnecting whenever the connection drops.
//!
//! ```sh
//! OBS_HOST=127.0.0.1 OBS_PASSWORD=... cargo run -p obs-websocket --example stream --features websocket
//! ```

use obs_websocket::{ObsConfig, ObsWebSocketManager, PollingConfig, RetryConfig};

#[tokio::main]
async fn main() {
	tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();

	let obs = ObsWebSocketManager::new(ObsConfig::from_env());
	let retry_delay = RetryConfig::default().initial_delay;

	tokio::select! {
		() = async {
			loop {
				match obs.connect(PollingConfig::default()).await {
					Ok(()) => {
						obs.stream_events(|event| Box::pin(async move { tracing::debug!(?event, "OBS event") })).await;
						tracing::warn!("OBS connection ended");
					}
					Err(e) => tracing::error!("failed to connect to OBS: {e}"),
				}
				tokio::time::sleep(retry_delay).await;
			}
		} => {}
		_ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
	}

	obs.disconnect().await;
}

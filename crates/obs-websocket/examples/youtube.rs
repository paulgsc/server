//! Point OBS at the `YouTube` ingest, start streaming, and report what OBS says.
//!
//! ```sh
//! OBS_HOST=127.0.0.1 OBS_PASSWORD=... YOUTUBE_STREAM_KEY=xxxx-xxxx-xxxx-xxxx-xxxx \
//!   cargo run -p obs-websocket --example youtube --features websocket
//! ```
//!
//! This only makes OBS push video to the stream key. Whether that shows up as
//! a live broadcast, and with what title or privacy, is decided on the `YouTube`
//! side (the broadcast bound to that key), not by OBS.

use obs_websocket::{ObsCommand, ObsConfig, ObsEvent, ObsWebSocketManager, PollingConfig, StreamKey};
use tokio::time::{timeout, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

	let stream_key = std::env::var("YOUTUBE_STREAM_KEY").map_err(|_| "set YOUTUBE_STREAM_KEY to your YouTube stream key")?;

	let obs = ObsWebSocketManager::new(ObsConfig::from_env());
	obs.connect(PollingConfig::none()).await?;
	tracing::info!("Connected to OBS");

	obs
		.execute_command(ObsCommand::SetYouTubeStream {
			stream_key: StreamKey::new(stream_key),
		})
		.await?;
	tracing::info!("OBS stream output now targets YouTube");

	// Subscribe first so the StreamStateChanged caused by StartStream can't be missed.
	let mut events = obs.events();

	// An error here carries OBS's own reason, e.g. that the output is already running.
	obs.execute_command(ObsCommand::StartStream).await?;
	tracing::info!("OBS accepted StartStream; waiting for the output to come up");

	// Accepting StartStream only means OBS began starting the output. The
	// ingest connection succeeding or failing arrives as StreamStateChanged.
	let outcome = timeout(Duration::from_secs(30), async {
		while let Ok(event) = events.recv().await {
			if let ObsEvent::StreamStateChanged(state) = event {
				tracing::info!(?state, "StreamStateChanged");
				match state.output_state.as_deref() {
					Some("OBS_WEBSOCKET_OUTPUT_STARTED") => return Ok(()),
					Some("OBS_WEBSOCKET_OUTPUT_STOPPED") => return Err("OBS stopped the output; check the stream key and OBS's log"),
					_ => {}
				}
			}
		}
		Err("OBS connection closed")
	})
	.await;

	match outcome {
		Ok(Ok(())) => tracing::info!("Streaming to YouTube"),
		Ok(Err(reason)) => tracing::error!("{reason}"),
		Err(_) => tracing::warn!("No StreamStateChanged within 30s"),
	}

	obs.disconnect().await;
	Ok(())
}

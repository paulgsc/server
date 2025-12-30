use crate::{ObsNatsService, Result};
use obs_websocket::PollingConfig;
use some_transport::Transport;
use std::sync::Arc;
use ws_events::{events::ObsStatusMessage, unified_event, UnifiedEvent};

impl ObsNatsService {
	/// Spawn task to bridge OBS events to NATS
	pub fn spawn_event_bridge(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			tracing::info!("🌉 Starting event bridge");

			loop {
				tokio::select! {
					_ = self.cancel_token.cancelled() => {
						tracing::info!("🛑 Event bridge shutting down");
						break;
					}
					result = self.connect_and_stream_events() => {
						match result {
							Ok(()) => {
								tracing::info!("Event streaming ended normally");
							}
							Err(e) => {
								tracing::error!("❌ Event bridge error: {}", e);
							}
						}

						// Disconnect cleanly
						let _ = self.obs_manager.disconnect().await;

						// Retry with exponential backoff
						let delay = self.calculate_retry_delay();
						tracing::info!("⏳ Retrying connection in {:?}", delay);

						tokio::select! {
							_ = self.cancel_token.cancelled() => {
								tracing::info!("🛑 Shutdown during retry delay");
								break;
							}
							_ = tokio::time::sleep(delay) => {
								continue;
							}
						}
					}
				}
			}

			tracing::info!("✅ Event bridge stopped");
		})
	}

	/// Connect to OBS and stream events to NATS
	async fn connect_and_stream_events(&self) -> Result<()> {
		tracing::info!("🔌 Connecting to OBS WebSocket");
		let polling_config = PollingConfig::default();
		self.obs_manager.connect(polling_config).await?;
		tracing::info!("✅ Connected to OBS, starting event stream");

		// Clone for the closure
		let event_transport = self.transport.clone();
		let event_subject = self.config.event_subject.clone();
		let cancel_token = self.cancel_token.clone();

		self
			.obs_manager
			.stream_events(move |obs_event| {
				let transport = event_transport.clone();
				let subject = event_subject.clone();
				let cancel = cancel_token.clone();

				Box::pin(async move {
					if cancel.is_cancelled() {
						return;
					}

					// Create the protobuf message, handling serialization errors
					match ObsStatusMessage::new(obs_event) {
						Ok(message) => {
							let unified_event = UnifiedEvent {
								event: Some(unified_event::Event::ObsStatus(message)),
							};
							if let Err(e) = transport.send_to_subject(&subject, unified_event).await {
								tracing::error!("❌ Failed to publish event: {}", e);
							}
						}
						Err(e) => {
							tracing::error!("❌ Failed to serialize event to protobuf: {}", e);
						}
					}
				})
			})
			.await?;

		Ok(())
	}
}

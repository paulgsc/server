use crate::websocket::{BroadcastOutcome, Event, EventType};
use crate::WebSocketFsm;
use obs_websocket::{ObsWebSocketManager, PollingConfig};
use std::sync::Arc;
use tokio::{task::JoinHandle, time::Duration};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

impl WebSocketFsm {
	/// Spawns a background task to bridge OBS events for a given `event_type`.
	/// Uses a shared Arc<Self> and supports clean cancellation.
	pub fn bridge_obs_events(self: Arc<Self>, event_type: EventType) -> JoinHandle<()> {
		// Cancellation token to gracefully stop the loop if needed
		let cancel_token = CancellationToken::new();
		let fsm = self.clone();

		tokio::spawn(async move {
			info!("Starting OBS bridge FSM (lazy subscriber-driven)");

			loop {
				tokio::select! {
					_ = cancel_token.cancelled() => {
						info!("Bridge task cancelled for {:?}", event_type);
						break;
					}
						_ = fsm.wait_for_subscriber_group(&event_type) => {
						info!("Subscriber(s) present for {:?} → connecting OBS", event_type);

						if let Err(e) = fsm.clone().run_obs_session(&fsm.obs_manager, &cancel_token, event_type.clone()).await {
							error!("OBS session ended with error: {e}");
						}

						info!("OBS session ended, waiting for new subscribers...");
					}
				}
			}
		})
	}

	async fn run_obs_session(self: Arc<Self>, obs_manager: &Arc<ObsWebSocketManager>, cancel_token: &CancellationToken, event_type: EventType) -> anyhow::Result<()> {
		let requests = PollingConfig::default();

		obs_manager.connect(requests).await?;
		let obs_manager_stream_task = obs_manager.clone();
		info!("Connected to OBS WebSocket");

		let stream_task = tokio::spawn({
			let metrics = self.metrics.clone();
			let connections = self.connections.clone();
			let ev_ty = event_type.clone();

			async move {
				obs_manager_stream_task
					.stream_events(|obs_event| {
						let metrics = metrics.clone();
						let connections = connections.clone();
						let ev_ty = ev_ty.clone();
						Box::pin(async move {
							// Only stream ObsStatus, even if ObsCommand subscribers exist
							if ev_ty.is_stream_origin() {
								let event = Event::ObsStatus { status: obs_event };
								let event_type_name = event.get_type();

								let outcome = Self::broadcast_event_to_subscribers(event, &event_type_name, connections.clone()).await;

								if let BroadcastOutcome::Completed { process_result } = outcome {
									metrics.broadcast_attempt(process_result.failed == 0);
									debug!("Event {:?}: {} delivered, {} failed", event_type_name, process_result.delivered, process_result.failed);
								}
							}
						})
					})
					.await
			}
		});

		// Wait until either cancelled or no subscribers for this event type
		let sub_notify = self.subscriber_notify.clone();
		loop {
			let has_subs = self.has_subscriber_for_group(&event_type);

			if !has_subs {
				info!("No subscribers for {:?} → disconnecting OBS", event_type);
				break;
			}

			tokio::select! {
				_ = cancel_token.cancelled() => {
					info!("Cancellation requested → disconnecting OBS");
					break;
				}
				_ = sub_notify.notified() => {
					// Re-evaluate subscription state
				}
				_ = tokio::time::sleep(Duration::from_secs(1)) => {
					// periodic check to avoid deadlock if notify missed
				}
			}
		}

		stream_task.abort();
		obs_manager.disconnect().await?;
		Ok(())
	}
}

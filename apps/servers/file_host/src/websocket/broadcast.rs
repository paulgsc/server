use crate::WebSocketFsm;
use some_transport::Transport;
use tokio::time::Instant;
use tracing::error;
use ws_events::{events::Event, UnifiedEvent};

mod errors;
mod handlers;
mod types;

use errors::BroadcastError;
use types::BroadcastResult;

impl WebSocketFsm {
	/// Broadcast an event (wrapper with metrics)
	pub async fn broadcast_event_tracked(&self, event: Event) -> BroadcastResult {
		let start = Instant::now();

		match self.broadcast_event(event).await {
			Ok(_count) => {
				let duration = start.elapsed();
				self.metrics.broadcast_attempt(true);
				BroadcastResult::success(0, duration)
			}
			Err(e) => {
				let duration = start.elapsed();
				self.metrics.broadcast_attempt(false);
				error!("Broadcast failed: {}", e);
				BroadcastResult {
					delivered: 0,
					failed: 1,
					duration,
				}
			}
		}
	}

	/// Broadcast an event to all subscribers of its type
	pub async fn broadcast_event(&self, event: Event) -> Result<(), BroadcastError> {
		let unified_event = UnifiedEvent::try_from(event.clone())?;
		let event_type = event.get_type().ok_or(BroadcastError::NoEventType)?;
		let subject = event_type.subject();

		self.transport.send_to_subject(subject, unified_event).await?;

		Ok(())
	}
}

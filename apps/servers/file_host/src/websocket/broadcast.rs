use crate::WebSocketFsm;
use some_transport::TransportError;
use tracing::{debug, error, info, warn};

mod handlers;
mod types;

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
	pub async fn broadcast_event(&self, event: Event) -> Result<usize, TransportError> {
		let event_type = event.event_type().ok_or("Event has no type");
		let subject = event_type.subject();

		self.transport.send_to_subject(subject, event).await?;
	}
}

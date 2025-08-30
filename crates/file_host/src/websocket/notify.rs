use crate::websocket::EventType;
use crate::WebSocketFsm;

impl WebSocketFsm {
	/// Wait until at least one connection is subscribed to the given event type
	pub async fn wait_for_subscriber(&self, event_type: EventType) {
		loop {
			let any_subs = self
				.connections
				.iter()
				.any(|entry| entry.value().is_active() && entry.value().is_subscribed_to(&event_type));
			if any_subs {
				return;
			}
			self.subscriber_notify.notified().await;
		}
	}

	/// Wait until no connection is subscribed to the given event type
	pub async fn wait_for_no_subscribers(&self, event_type: EventType) {
		loop {
			let any_subs = self
				.connections
				.iter()
				.any(|entry| entry.value().is_active() && entry.value().is_subscribed_to(&event_type));
			if !any_subs {
				return;
			}
			self.subscriber_notify.notified().await;
		}
	}

	/// Call after any subscription change
	pub fn update_subscriber_state(&self) {
		self.subscriber_notify.notify_waiters();
	}
}

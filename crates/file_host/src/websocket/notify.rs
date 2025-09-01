use crate::websocket::EventType;
use crate::WebSocketFsm;

impl WebSocketFsm {
	/// Returns true if any subscriber exists for any EventType in the group
	pub fn has_subscriber_for_group(&self, event_type: &EventType) -> bool {
		let group = event_type.lazy_trigger_group();
		self.connections.iter().any(|entry| {
			let conn = entry.value();
			conn.is_active() && group.iter().any(|et| conn.is_subscribed_to(et))
		})
	}

	/// Wait for any subscriber in the group
	pub async fn wait_for_subscriber_group(&self, event_type: &EventType) {
		loop {
			if self.has_subscriber_for_group(event_type) {
				return;
			}
			self.subscriber_notify.notified().await;
		}
	}

	/// Wait until no subscribers in the group
	pub async fn wait_for_no_subscribers_group(&self, event_type: &EventType) {
		loop {
			if !self.has_subscriber_for_group(event_type) {
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

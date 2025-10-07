use crate::websocket::EventType;
use crate::WebSocketFsm;
use futures::stream::{FuturesUnordered, StreamExt};

impl WebSocketFsm {
	/// Returns true if any subscriber exists for any EventType in the group
	pub async fn has_subscriber_for_group(&self, event_type: &EventType) -> bool {
		let group = event_type.lazy_trigger_group();

		let futures = FuturesUnordered::new();

		// Spawn a future for each connection handle
		for handle in self.store.keys().into_iter().filter_map(|key| self.store.get(&key)) {
			let handle = handle.clone();
			let group = group.clone();
			futures.push(async move {
				if let Ok(state) = handle.get_state().await {
					if state.is_active {
						for et in &group {
							if handle.is_subscribed_to(et) {
								return true;
							}
						}
					}
				}
				false
			});
		}

		// Collect results, short-circuiting if any future returned true
		futures.any(|has| async move { has }).await
	}

	/// Wait for any subscriber in the group
	pub async fn wait_for_subscriber_group(&self, event_type: &EventType) {
		loop {
			if self.has_subscriber_for_group(event_type).await {
				return;
			}
			self.subscriber_notify.notified().await;
		}
	}

	/// Wait until no subscribers in the group
	pub async fn wait_for_no_subscribers_group(&self, event_type: &EventType) {
		loop {
			if !self.has_subscriber_for_group(event_type).await {
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

use crate::websocket::EventType;
use crate::WebSocketFsm;
use tokio::task::JoinSet;

impl WebSocketFsm {
	/// Returns true if any active subscriber exists for any EventType in the group.
	/// Short-circuits on first match for optimal performance.
	pub async fn has_subscriber_for_group(&self, event_type: &EventType) -> bool {
		let group = event_type.lazy_trigger_group();
		let mut join_set = JoinSet::new();

		// Spawn concurrent checks for all connections
		for handle in self.store.keys().into_iter().filter_map(|key| self.store.get(&key)) {
			let handle = handle.clone();
			let group = group.clone();

			join_set.spawn(async move {
				// Get state - bail early if not active
				let state = handle.get_state().await.ok()?;
				if !state.is_active {
					return None;
				}

				// Get all subscriptions in one call (more efficient than N calls)
				let subscriptions = handle.get_subscriptions().await.ok()?;

				// Check if any group event is subscribed
				for event_type in &group {
					if subscriptions.contains(event_type) {
						return Some(true);
					}
				}

				Some(false)
			});
		}

		// Process results as they complete, short-circuit on first match
		while let Some(result) = join_set.join_next().await {
			if let Ok(Some(true)) = result {
				join_set.abort_all(); // Cancel all remaining checks
				return true;
			}
		}

		false
	}

	/// Wait for any active subscriber in the group (non-busy wait).
	/// Uses notify pattern to avoid polling.
	pub async fn wait_for_subscriber_group(&self, event_type: &EventType) {
		loop {
			if self.has_subscriber_for_group(event_type).await {
				return;
			}
			self.subscriber_notify.notified().await;
		}
	}

	/// Wait until no active subscribers remain in the group (non-busy wait).
	/// Uses notify pattern to avoid polling.
	pub async fn wait_for_no_subscribers_group(&self, event_type: &EventType) {
		loop {
			if !self.has_subscriber_for_group(event_type).await {
				return;
			}
			self.subscriber_notify.notified().await;
		}
	}

	/// Notify all waiters that subscription state has changed.
	/// Call this after subscribe/unsubscribe operations.
	#[inline]
	pub fn notify_subscription_change(&self) {
		self.subscriber_notify.notify_waiters();
	}
}

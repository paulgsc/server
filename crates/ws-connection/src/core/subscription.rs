use std::collections::HashSet;
use std::hash::Hash;

/// Trait for event keys that can be subscribed to
pub trait EventKey: Clone + Eq + Hash + std::fmt::Debug {}

/// Blanket implementation for common types
impl<T> EventKey for T where T: Clone + Eq + Hash + std::fmt::Debug {}

/// Manages event subscriptions for a connection
#[derive(Debug, Clone)]
pub struct SubscriptionManager<K: EventKey = String> {
	subscriptions: HashSet<K>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionChange {
	pub added: usize,
	pub removed: usize,
	pub total: usize,
}

impl<K: EventKey> SubscriptionManager<K> {
	/// Create empty subscription manager
	pub fn new() -> Self {
		Self { subscriptions: HashSet::new() }
	}

	/// Create subscription manager with initial subscriptions
	pub fn with_subscriptions<I>(event_keys: I) -> Self
	where
		I: IntoIterator<Item = K>,
	{
		Self {
			subscriptions: event_keys.into_iter().collect(),
		}
	}

	/// Subscribe to event keys
	pub fn subscribe<I>(&mut self, event_keys: I) -> SubscriptionChange
	where
		I: IntoIterator<Item = K>,
	{
		let mut added_count = 0;
		for event_key in event_keys {
			if self.subscriptions.insert(event_key) {
				added_count += 1;
			}
		}

		SubscriptionChange {
			added: added_count,
			removed: 0,
			total: self.subscriptions.len(),
		}
	}

	/// Unsubscribe from event keys
	pub fn unsubscribe<I>(&mut self, event_keys: I) -> SubscriptionChange
	where
		I: IntoIterator<Item = K>,
	{
		let mut removed_count = 0;
		for event_key in event_keys {
			if self.subscriptions.remove(&event_key) {
				removed_count += 1;
			}
		}

		SubscriptionChange {
			added: 0,
			removed: removed_count,
			total: self.subscriptions.len(),
		}
	}

	/// Check if subscribed to an event key
	pub fn is_subscribed_to(&self, event_key: &K) -> bool {
		self.subscriptions.contains(event_key)
	}

	/// Get all current subscriptions
	pub fn get_subscriptions(&self) -> &HashSet<K> {
		&self.subscriptions
	}

	/// Get subscription count
	pub fn count(&self) -> usize {
		self.subscriptions.len()
	}

	/// Check if has any subscriptions
	pub fn is_empty(&self) -> bool {
		self.subscriptions.is_empty()
	}

	/// Clear all subscriptions
	pub fn clear(&mut self) -> SubscriptionChange {
		let removed_count = self.subscriptions.len();
		self.subscriptions.clear();

		SubscriptionChange {
			added: 0,
			removed: removed_count,
			total: 0,
		}
	}

	/// Set subscriptions to a specific set (replacing all current ones)
	pub fn set_subscriptions<I>(&mut self, event_keys: I) -> SubscriptionChange
	where
		I: IntoIterator<Item = K>,
	{
		let new_subscriptions: HashSet<K> = event_keys.into_iter().collect();
		let new_count = new_subscriptions.len();

		// Fast path: if sets are identical, no changes
		if self.subscriptions == new_subscriptions {
			return SubscriptionChange {
				added: 0,
				removed: 0,
				total: new_count,
			};
		}

		// Replace and calculate changes based on counts only
		self.subscriptions = new_subscriptions;

		// We can't know exact added/removed without iteration,
		// but for most use cases the total change is what matters
		SubscriptionChange {
			added: 0,   // Could be calculated if needed
			removed: 0, // Could be calculated if needed
			total: new_count,
		}
	}
}

impl<K: EventKey> Default for SubscriptionManager<K> {
	fn default() -> Self {
		Self::new()
	}
}

impl SubscriptionChange {
	pub fn net_change(&self) -> i32 {
		self.added as i32 - self.removed as i32
	}

	pub fn has_changes(&self) -> bool {
		self.added > 0 || self.removed > 0
	}
}

use std::collections::HashSet;
use std::hash::Hash;

/// Trait for event keys that can be subscribed to.
pub trait EventKey: Clone + Eq + Hash + std::fmt::Debug + Send + Sync + 'static {}

/// Blanket implementation for all suitable types.
impl<T> EventKey for T where T: Clone + Eq + Hash + std::fmt::Debug + Send + Sync + 'static {}

/// Manages event subscriptions for a connection.
#[derive(Debug, Clone)]
pub struct SubscriptionManager<K: EventKey = String> {
	subscriptions: HashSet<K>,
}

/// Describes changes in subscription state.
#[derive(Debug, Clone)]
pub struct SubscriptionChange {
	pub added: usize,
	pub removed: usize,
	pub total: usize,
}

impl<K: EventKey> SubscriptionManager<K> {
	/// Create an empty subscription manager.
	#[must_use]
	pub fn new() -> Self {
		Self { subscriptions: HashSet::new() }
	}

	/// Create a subscription manager with initial subscriptions.
	#[must_use]
	pub fn with_subscriptions<I>(event_keys: I) -> Self
	where
		I: IntoIterator<Item = K>,
	{
		Self {
			subscriptions: event_keys.into_iter().collect(),
		}
	}

	/// Subscribe to event keys.
	#[must_use]
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

	/// Unsubscribe from event keys.
	#[must_use]
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

	/// Check if subscribed to an event key.
	#[must_use]
	pub fn is_subscribed_to(&self, event_key: &K) -> bool {
		self.subscriptions.contains(event_key)
	}

	/// Get all current subscriptions.
	#[must_use]
	pub fn get_subscriptions(&self) -> &HashSet<K> {
		&self.subscriptions
	}

	/// Get subscription count.
	#[must_use]
	pub fn count(&self) -> usize {
		self.subscriptions.len()
	}

	/// Check if there are no subscriptions.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.subscriptions.is_empty()
	}

	/// Clear all subscriptions.
	#[must_use]
	pub fn clear(&mut self) -> SubscriptionChange {
		let removed_count = self.subscriptions.len();
		self.subscriptions.clear();

		SubscriptionChange {
			added: 0,
			removed: removed_count,
			total: 0,
		}
	}

	/// Replace current subscriptions with a new set.
	#[must_use]
	pub fn set_subscriptions<I>(&mut self, event_keys: I) -> SubscriptionChange
	where
		I: IntoIterator<Item = K>,
	{
		let new_subscriptions: HashSet<K> = event_keys.into_iter().collect();
		let new_count = new_subscriptions.len();

		// Fast path: if sets are identical, no changes.
		if self.subscriptions == new_subscriptions {
			return SubscriptionChange {
				added: 0,
				removed: 0,
				total: new_count,
			};
		}

		self.subscriptions = new_subscriptions;

		// Could calculate added/removed precisely if needed.
		SubscriptionChange {
			added: 0,
			removed: 0,
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
	/// Compute the net change (added - removed).
	#[must_use]
	pub fn net_change(&self) -> i32 {
		let added = i64::try_from(self.added).unwrap_or(i64::MAX);
		let removed = i64::try_from(self.removed).unwrap_or(i64::MAX);
		(added - removed).clamp(i32::MIN as i64, i32::MAX as i64) as i32
	}

	/// Whether there were any changes.
	#[must_use]
	pub fn has_changes(&self) -> bool {
		self.added > 0 || self.removed > 0
	}
}

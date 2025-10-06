use crate::core::subscription::{EventKey, SubscriptionChange, SubscriptionManager};
use crate::types::{ClientId, ConnectionId};
use std::{
	net::SocketAddr,
	time::{Duration, Instant},
};

/// Pure connection domain object - no side effects, metrics, or logging
/// Generic over event key type K for compile-time type safety
#[derive(Debug)]
pub struct Connection<K: EventKey = String> {
	pub id: ConnectionId,
	pub client_id: ClientId,
	pub established_at: Instant,
	pub source_addr: SocketAddr,
	pub subscriptions: SubscriptionManager<K>,
}

impl<K: EventKey> Connection<K> {
	/// Create a new connection with default subscriptions
	pub fn new(client_id: ClientId, source_addr: SocketAddr) -> Self {
		let now = Instant::now();
		Self {
			id: ConnectionId::new(),
			client_id,
			established_at: now,
			source_addr,
			subscriptions: SubscriptionManager::new(),
		}
	}

	/// Subscribe to event types
	pub fn subscribe<I>(&mut self, event_types: I) -> SubscriptionChange
	where
		I: IntoIterator<Item = K>,
	{
		self.subscriptions.subscribe(event_types)
	}

	/// Check if subscribed to an event type
	pub fn is_subscribed_to(&self, event_type: &K) -> bool {
		self.subscriptions.is_subscribed_to(event_type)
	}

	/// Get connection duration
	pub fn get_duration(&self) -> Duration {
		self.established_at.elapsed()
	}

	/// Get subscription count
	pub fn get_subscription_count(&self) -> usize {
		self.subscriptions.count()
	}

	/// Get all subscriptions
	pub fn get_subscriptions(&self) -> std::collections::HashSet<K> {
		self.subscriptions.get_subscriptions().clone()
	}
}

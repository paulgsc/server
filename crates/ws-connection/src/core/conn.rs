// src/conn.rs
use crate::core::subscription::{EventKey, SubscriptionChange, SubscriptionManager};
use crate::types::{ClientId, ConnectionId, ConnectionState};
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
	pub state: ConnectionState,
	pub source_addr: SocketAddr,
	// Private fields managed internally
	subscriptions: SubscriptionManager<K>,
	last_activity: Instant,
}

impl<K: EventKey> Connection<K> {
	/// Create a new connection with default subscriptions
	pub fn new(client_id: ClientId, source_addr: SocketAddr) -> Self {
		let now = Instant::now();
		Self {
			id: ConnectionId::new(),
			client_id,
			established_at: now,
			state: ConnectionState::Active { last_ping: now },
			source_addr,
			subscriptions: SubscriptionManager::new(),
			last_activity: now,
		}
	}

	/// Pure state transition: record ping / activity
	pub fn record_activity(&mut self) {
		self.last_activity = Instant::now();
		if let ConnectionState::Active { ref mut last_ping } = self.state {
			*last_ping = self.last_activity;
		}
	}

	/// Subscribe to event types
	pub fn subscribe<I>(&mut self, event_types: I) -> SubscriptionChange
	where
		I: IntoIterator<Item = K>,
	{
		self.subscriptions.subscribe(event_types)
	}

	/// Unsubscribe from event types
	pub fn unsubscribe<I>(&mut self, event_types: I) -> SubscriptionChange
	where
		I: IntoIterator<Item = K>,
	{
		self.subscriptions.unsubscribe(event_types)
	}

	/// Mark connection as stale
	pub fn mark_stale(&mut self, reason: String) {
		if let ConnectionState::Active { last_ping } = self.state {
			self.state = ConnectionState::Stale { last_ping, reason };
		}
	}

	/// Disconnect connection
	pub fn disconnect(&mut self, reason: String) {
		self.state = ConnectionState::Disconnected {
			reason,
			disconnected_at: Instant::now(),
		};
	}

	/// Check if connection is active
	pub fn is_active(&self) -> bool {
		matches!(self.state, ConnectionState::Active { .. })
	}

	/// Check if connection should be marked as stale based on timeout
	pub fn should_be_stale(&self, timeout: Duration) -> bool {
		match &self.state {
			ConnectionState::Active { last_ping } => Instant::now().duration_since(*last_ping) > timeout,
			_ => false,
		}
	}

	/// Check if connection is in stale state
	pub fn is_stale(&self) -> bool {
		matches!(self.state, ConnectionState::Stale { .. })
	}

	/// Check if connection is disconnected
	pub fn is_disconnected(&self) -> bool {
		matches!(self.state, ConnectionState::Disconnected { .. })
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

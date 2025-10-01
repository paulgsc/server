use crate::core::conn::Connection;
use crate::core::subscription::EventKey;
use crate::types::ConnectionState;
use dashmap::DashMap;
use std::sync::Arc;

/// Generic, async-friendly connection store
#[derive(Debug, Clone)]
pub struct ConnectionStore<K: EventKey = String> {
	connections: Arc<DashMap<String, Arc<Connection<K>>>>,
}

impl<K: EventKey> ConnectionStore<K> {
	/// Create a new store
	pub fn new() -> Self {
		Self {
			connections: Arc::new(DashMap::new()),
		}
	}

	/// Insert or replace a connection
	pub fn insert(&self, key: String, connection: Connection<K>) -> Option<Arc<Connection<K>>> {
		let arc_conn = Arc::new(connection);
		self.connections.insert(key, arc_conn)
	}

	/// Get a connection cheaply (cloning the Arc)
	pub fn get(&self, key: &str) -> Option<Arc<Connection<K>>> {
		self.connections.get(key).map(|entry| Arc::clone(entry.value()))
	}

	/// Remove a connection
	pub fn remove(&self, key: &str) -> Option<Arc<Connection<K>>> {
		self.connections.remove(key).map(|(_, conn)| conn)
	}

	/// Count of connections
	pub fn len(&self) -> usize {
		self.connections.len()
	}

	/// Check if store is empty
	pub fn is_empty(&self) -> bool {
		self.connections.is_empty()
	}

	/// Get all keys
	pub fn keys(&self) -> Vec<String> {
		self.connections.iter().map(|entry| entry.key().clone()).collect()
	}

	/// Gather connection stats
	pub fn stats(&self) -> ConnectionStoreStats {
		let mut active = 0;
		let mut stale = 0;
		let mut disconnected = 0;
		let mut unique_clients = std::collections::HashSet::new();

		for entry in self.connections.iter() {
			let conn = entry.value();
			unique_clients.insert(conn.client_id.clone());

			match conn.state {
				ConnectionState::Active { .. } => active += 1,
				ConnectionState::Stale { .. } => stale += 1,
				ConnectionState::Disconnected { .. } => disconnected += 1,
			}
		}

		ConnectionStoreStats {
			total_connections: self.connections.len(),
			active_connections: active,
			stale_connections: stale,
			disconnected_connections: disconnected,
			unique_clients: unique_clients.len(),
		}
	}
}

/// Simple store statistics
#[derive(Debug, Clone)]
pub struct ConnectionStoreStats {
	pub total_connections: usize,
	pub active_connections: usize,
	pub stale_connections: usize,
	pub disconnected_connections: usize,
	pub unique_clients: usize,
}

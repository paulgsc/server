use dashmap::DashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ConnectionStore {
	connections: Arc<DashMap<String, Connection>>,
}

impl ConnectionStore {
	pub fn new() -> Self {
		Self {
			connections: Arc::new(DashMap::new()),
		}
	}

	pub fn insert(&self, key: String, connection: Connection) -> Option<Connection> {
		self.connections.insert(key, connection)
	}

	pub fn get(&self, key: &str) -> Option<Connection> {
		self.connections.get(key).map(|entry| entry.value().clone())
	}

	pub fn remove(&self, key: &str) -> Option<Connection> {
		self.connections.remove(key).map(|(_, conn)| conn)
	}

	pub fn len(&self) -> usize {
		self.connections.len()
	}

	pub fn is_empty(&self) -> bool {
		self.connections.is_empty()
	}

	pub fn keys(&self) -> Vec<String> {
		self.connections.iter().map(|entry| entry.key().clone()).collect()
	}

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

#[derive(Debug, Clone)]
pub struct ConnectionStoreStats {
	pub total_connections: usize,
	pub active_connections: usize,
	pub stale_connections: usize,
	pub disconnected_connections: usize,
	pub unique_clients: usize,
}

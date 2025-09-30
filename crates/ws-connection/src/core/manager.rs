// src/websocket/store.rs

use crate::websocket::{connection::Connection, errors::*, types::*};
use async_trait::async_trait;
use dashmap::DashMap;
use std::{collections::HashMap, sync::Arc};

/// Abstraction over connection storage for testing and flexibility
#[async_trait]
pub trait ConnectionStore: Send + Sync {
	/// Insert a new connection
	fn insert(&self, key: String, connection: Connection) -> Option<Connection>;

	/// Get a connection by key (read-only)
	fn get(&self, key: &str) -> Option<Connection>;

	/// Get connections by client ID
	fn get_by_client(&self, client_id: &ClientId) -> Vec<(String, Connection)>;

	/// Remove a connection
	fn remove(&self, key: &str) -> Option<Connection>;

	/// Get connection count
	fn len(&self) -> usize;

	/// Check if store is empty
	fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// Get all connection keys
	fn keys(&self) -> Vec<String>;

	/// Iterate over connections with a predicate
	fn find_matching<F>(&self, predicate: F) -> Vec<String>
	where
		F: Fn(&Connection) -> bool;

	/// Get store statistics
	fn get_stats(&self) -> ConnectionStoreStats;

	/// Apply operation to a connection if it exists
	fn with_connection<F, R>(&self, key: &str, f: F) -> Result<R, ConnectionError>
	where
		F: FnOnce(&Connection) -> R;

	/// Apply mutable operation to a connection if it exists
	fn with_connection_mut<F, R>(&self, key: &str, f: F) -> Result<R, ConnectionError>
	where
		F: FnOnce(&mut Connection) -> R;
}

#[derive(Debug, Clone)]
pub struct ConnectionStoreStats {
	pub total_connections: usize,
	pub active_connections: usize,
	pub stale_connections: usize,
	pub disconnected_connections: usize,
	pub unique_clients: usize,
}

/// DashMap-based implementation of ConnectionStore
pub struct DashMapConnectionStore {
	connections: Arc<DashMap<String, Connection>>,
}

impl DashMapConnectionStore {
	pub fn new() -> Self {
		Self {
			connections: Arc::new(DashMap::new()),
		}
	}

	/// Get reference to underlying DashMap for advanced operations
	pub fn raw(&self) -> &DashMap<String, Connection> {
		&self.connections
	}
}

impl Default for DashMapConnectionStore {
	fn default() -> Self {
		Self::new()
	}
}

#[async_trait]
impl ConnectionStore for DashMapConnectionStore {
	fn insert(&self, key: String, connection: Connection) -> Option<Connection> {
		self.connections.insert(key, connection)
	}

	fn get(&self, key: &str) -> Option<Connection> {
		self.connections.get(key).map(|entry| entry.value().clone())
	}

	fn get_by_client(&self, client_id: &ClientId) -> Vec<(String, Connection)> {
		self
			.connections
			.iter()
			.filter(|entry| &entry.value().client_id == client_id)
			.map(|entry| (entry.key().clone(), entry.value().clone()))
			.collect()
	}

	fn remove(&self, key: &str) -> Option<Connection> {
		self.connections.remove(key).map(|(_, connection)| connection)
	}

	fn len(&self) -> usize {
		self.connections.len()
	}

	fn keys(&self) -> Vec<String> {
		self.connections.iter().map(|entry| entry.key().clone()).collect()
	}

	fn find_matching<F>(&self, predicate: F) -> Vec<String>
	where
		F: Fn(&Connection) -> bool,
	{
		self.connections.iter().filter(|entry| predicate(entry.value())).map(|entry| entry.key().clone()).collect()
	}

	fn get_stats(&self) -> ConnectionStoreStats {
		let connections = futures::executor::block_on(self.connections.read());
		let mut active = 0;
		let mut stale = 0;
		let mut disconnected = 0;
		let mut unique_clients = std::collections::HashSet::new();

		for (_, conn) in connections.iter() {
			unique_clients.insert(conn.client_id.clone());

			match conn.state {
				ConnectionState::Active { .. } => active += 1,
				ConnectionState::Stale { .. } => stale += 1,
				ConnectionState::Disconnected { .. } => disconnected += 1,
			}
		}

		ConnectionStoreStats {
			total_connections: connections.len(),
			active_connections: active,
			stale_connections: stale,
			disconnected_connections: disconnected,
			unique_clients: unique_clients.len(),
		}
	}

	fn with_connection<F, R>(&self, key: &str, f: F) -> Result<R, ConnectionError>
	where
		F: FnOnce(&Connection) -> R,
	{
		let connections = futures::executor::block_on(self.connections.read());
		match connections.get(key) {
			Some(conn) => Ok(f(conn)),
			None => Err(ConnectionError::NotFound),
		}
	}

	fn with_connection_mut<F, R>(&self, key: &str, f: F) -> Result<R, ConnectionError>
	where
		F: FnOnce(&mut Connection) -> R,
	{
		let mut connections = futures::executor::block_on(self.connections.write());
		match connections.get_mut(key) {
			Some(conn) => Ok(f(conn)),
			None => Err(ConnectionError::NotFound),
		}
	}
}

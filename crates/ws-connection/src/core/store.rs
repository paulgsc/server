use crate::actor::ConnectionHandle;
use crate::core::conn::Connection;
use crate::core::subscription::EventKey;
use dashmap::DashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ConnectionStore<K: EventKey = String> {
	handles: Arc<DashMap<String, ConnectionHandle<K>>>,
}

impl<K: EventKey> ConnectionStore<K> {
	pub fn new() -> Self {
		Self {
			handles: Arc::new(DashMap::new()),
		}
	}

	/// Insert connection handle and spawn its actor
	pub fn insert(&self, key: String, connection: Connection<K>) -> ConnectionHandle<K> {
		let (handle, actor) = ConnectionHandle::new(connection, 100);

		// Spawn the actor
		tokio::spawn(actor.run());

		self.handles.insert(key, handle.clone());
		handle
	}

	/// Get connection handle
	pub fn get(&self, key: &str) -> Option<ConnectionHandle<K>> {
		self.handles.get(key).map(|entry| entry.value().clone())
	}

	/// Remove connection and shutdown its actor
	pub async fn remove(&self, key: &str) -> Option<ConnectionHandle<K>> {
		if let Some((_, handle)) = self.handles.remove(key) {
			let _ = handle.shutdown().await;
			Some(handle)
		} else {
			None
		}
	}

	pub fn len(&self) -> usize {
		self.handles.len()
	}

	pub fn is_empty(&self) -> bool {
		self.handles.is_empty()
	}

	pub fn keys(&self) -> Vec<String> {
		self.handles.iter().map(|entry| entry.key().clone()).collect()
	}

	/// Get stats by querying all actors
	pub async fn stats(&self) -> ConnectionStoreStats {
		let mut active = 0;
		let mut stale = 0;
		let mut disconnected = 0;
		let mut unique_clients = std::collections::HashSet::new();

		for entry in self.handles.iter() {
			let handle = entry.value();
			unique_clients.insert(handle.connection.client_id.clone());

			if let Ok(state) = handle.get_state().await {
				if state.is_active {
					active += 1;
				} else if state.is_stale {
					stale += 1;
				} else {
					disconnected += 1;
				}
			}
		}

		ConnectionStoreStats {
			total_connections: self.handles.len(),
			active_connections: active,
			stale_connections: stale,
			disconnected_connections: disconnected,
			unique_clients: unique_clients.len(),
		}
	}

	/// Batch operation: send command to all matching connections
	pub async fn for_each<F>(&self, mut f: F)
	where
		F: FnMut(&ConnectionHandle<K>),
	{
		for entry in self.handles.iter() {
			f(entry.value());
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

impl<K: EventKey> Default for ConnectionStore<K> {
	fn default() -> Self {
		Self::new()
	}
}

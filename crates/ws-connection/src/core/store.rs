use crate::actor::ConnectionHandle;
use crate::core::conn::Connection;
use crate::core::subscription::EventKey;
use dashmap::DashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

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
	pub fn insert(self: &Arc<Self>, key: String, connection: Connection, parent_token: &CancellationToken) -> ConnectionHandle<K> {
		let (handle, actor, token) = ConnectionHandle::new(connection, 100, parent_token);
		let store = self.clone();

		tokio::spawn({
			let key = key.clone();
			async move {
				tokio::select! {
					_ = actor.run() => {
						tracing::info!("actor {key} finished normally");
					}
					_ = token.cancelled() => {
						tracing::info!("actor {key} received cancellation");
						store.remove(&key).await; // graceful cleanup if implemented
					}
				}
			}
		});

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

	/// Execute an async closure for each connection handle.
	/// The closure is executed for all handles, but not concurrently.
	pub async fn for_each_async<F, Fut>(&self, mut f: F)
	where
		F: FnMut(ConnectionHandle<K>) -> Fut,
		Fut: std::future::Future<Output = ()>,
	{
		// Clone handles to avoid holding DashMap locks across await
		let handles: Vec<_> = self.handles.iter().map(|entry| entry.value().clone()).collect();

		for handle in handles {
			f(handle).await;
		}
	}

	/// Execute an async closure for each connection handle and return true
	/// if any closure invocation returns true. Stops early on first true.
	pub async fn for_any_async<F, Fut>(&self, mut f: F) -> bool
	where
		F: FnMut(ConnectionHandle<K>) -> Fut,
		Fut: std::future::Future<Output = bool>,
	{
		// Clone handles to avoid holding DashMap locks across await
		let handles: Vec<_> = self.handles.iter().map(|entry| entry.value().clone()).collect();

		for handle in handles {
			if f(handle).await {
				return true;
			}
		}

		false
	}

	/// Get stats by querying all actors (with concurrent queries for speed)
	pub async fn stats(&self) -> ConnectionStoreStats {
		use tokio::task::JoinSet;

		let mut join_set = JoinSet::new();
		let mut client_ids = Vec::with_capacity(self.handles.len());

		// Spawn concurrent state queries
		for entry in self.handles.iter() {
			let handle = entry.value().clone();
			client_ids.push(handle.connection.client_id.clone()); // Already cached!
			join_set.spawn(async move { handle.get_state().await });
		}

		// Collect results
		let mut active = 0;
		let mut stale = 0;
		let mut disconnected = 0;

		while let Some(result) = join_set.join_next().await {
			match result {
				Ok(Ok(state)) => {
					if state.is_active {
						active += 1;
					} else if state.is_stale {
						stale += 1;
					} else {
						disconnected += 1;
					}
				}
				_ => {
					// Actor unavailable or task panicked
					disconnected += 1;
				}
			}
		}

		let unique_clients: std::collections::HashSet<_> = client_ids.into_iter().collect();

		ConnectionStoreStats {
			total_connections: self.handles.len(),
			active_connections: active,
			stale_connections: stale,
			disconnected_connections: disconnected,
			unique_clients: unique_clients.len(),
		}
	}

	/// Batch operation: send command to all matching connections
	pub async fn for_each<F, Fut>(&self, f: F)
	where
		F: FnMut(ConnectionHandle<K>) -> Fut,
		Fut: std::future::Future<Output = ()>,
	{
		self.for_each_async(f).await;
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

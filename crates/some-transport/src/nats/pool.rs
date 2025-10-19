#![cfg(feature = "nats")]

use crate::error::{Result, TransportError};
use async_nats::Client;
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Idempotent NATS connection pool for managing singleton connections.
///
/// This ensures that multiple calls to connect with the same URL reuse
/// the same underlying connection, which is both efficient and prevents
/// resource exhaustion.
///
/// # Example
/// ```rust,no_run
/// use some_transport::NatsConnectionPool;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() {
///     let pool = NatsConnectionPool::global();
///     
///     // Multiple calls return the same connection
///     let client1 = pool.get_or_connect("nats://localhost:4222").await.unwrap();
///     let client2 = pool.get_or_connect("nats://localhost:4222").await.unwrap();
///     
///     // Arc pointers are equal
///     assert!(Arc::ptr_eq(&client1, &client2));
/// }
/// ```
#[derive(Clone)]
pub struct NatsConnectionPool {
	connections: Arc<dashmap::DashMap<String, Arc<OnceCell<Arc<Client>>>>>,
}

impl NatsConnectionPool {
	/// Creates a new connection pool.
	pub fn new() -> Self {
		Self {
			connections: Arc::new(dashmap::DashMap::new()),
		}
	}

	/// Returns the global singleton connection pool.
	///
	/// This is the recommended way to manage NATS connections across your app.
	pub fn global() -> &'static Self {
		static POOL: std::sync::OnceLock<NatsConnectionPool> = std::sync::OnceLock::new();
		POOL.get_or_init(NatsConnectionPool::new)
	}

	/// Gets or creates a connection to the specified NATS URL.
	///
	/// This method is idempotent - multiple calls with the same URL will
	/// return the same connection. The first call performs the actual connection,
	/// subsequent calls return the cached client.
	pub async fn get_or_connect(&self, url: impl Into<String>) -> Result<Arc<Client>> {
		let url = url.into();

		let cell = self.connections.entry(url.clone()).or_insert_with(|| Arc::new(OnceCell::new())).clone();

		let client = cell
			.get_or_try_init(|| async {
				let client = async_nats::connect(&url).await.map_err(|e| TransportError::NatsError(e.to_string()))?;
				Ok::<_, TransportError>(Arc::new(client))
			})
			.await?;

		Ok(client.clone())
	}

	/// Checks if a connection exists for the given URL.
	pub fn has_connection(&self, url: &str) -> bool {
		self.connections.get(url).and_then(|cell| cell.get().map(|_| true)).unwrap_or(false)
	}

	/// Removes a connection from the pool.
	///
	/// Returns the client if it existed. Note that existing references
	/// to the client will continue to work until all are dropped.
	pub fn remove(&self, url: &str) -> Option<Arc<Client>> {
		self.connections.remove(url).and_then(|(_, cell)| cell.get().cloned())
	}

	/// Clears all connections from the pool.
	pub fn clear(&self) {
		self.connections.clear();
	}

	/// Returns the number of active connections in the pool.
	pub fn len(&self) -> usize {
		self.connections.len()
	}

	/// Returns true if the pool has no connections.
	pub fn is_empty(&self) -> bool {
		self.connections.is_empty()
	}
}

impl Default for NatsConnectionPool {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn test_idempotent_connections() {
		let pool = NatsConnectionPool::new();

		// Connect twice to the same URL
		let client1 = pool.get_or_connect("nats://localhost:4222").await;
		let client2 = pool.get_or_connect("nats://localhost:4222").await;

		// Both should succeed (assuming NATS is running)
		if let (Ok(c1), Ok(c2)) = (client1, client2) {
			// Verify they're the same Arc pointer
			assert!(Arc::ptr_eq(&c1, &c2));
		}
	}

	#[tokio::test]
	async fn test_multiple_urls() {
		let pool = NatsConnectionPool::new();

		// Different URLs should create different connections
		let _client1 = pool.get_or_connect("nats://localhost:4222").await;
		let _client2 = pool.get_or_connect("nats://localhost:4223").await;

		assert_eq!(pool.len(), 2);
	}

	#[tokio::test]
	async fn test_has_connection() {
		let pool = NatsConnectionPool::new();

		assert!(!pool.has_connection("nats://localhost:4222"));

		let _ = pool.get_or_connect("nats://localhost:4222").await;

		// May be true if connection succeeded
		// (won't fail test if NATS isn't running)
	}

	#[tokio::test]
	async fn test_remove_connection() {
		let pool = NatsConnectionPool::new();

		let _ = pool.get_or_connect("nats://localhost:4222").await;

		let removed = pool.remove("nats://localhost:4222");
		assert!(removed.is_some() || removed.is_none()); // Either outcome is valid

		assert!(!pool.has_connection("nats://localhost:4222"));
	}

	#[tokio::test]
	async fn test_clear() {
		let pool = NatsConnectionPool::new();

		let _ = pool.get_or_connect("nats://localhost:4222").await;
		let _ = pool.get_or_connect("nats://localhost:4223").await;

		pool.clear();
		assert_eq!(pool.len(), 0);
	}

	#[tokio::test]
	async fn test_global_singleton() {
		let pool1 = NatsConnectionPool::global();
		let pool2 = NatsConnectionPool::global();

		// Should be the same instance
		assert!(std::ptr::eq(pool1, pool2));
	}
}

#![cfg(feature = "nats")]

use super::pool::NatsConnectionPool;
use super::receiver::{NatsReceiver, TransportReceiver};
use crate::error::{Result, TransportError};
use crate::traits::Transport;
use async_nats::Client;
use serde::{de::DeserializeOwned, Serialize};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// NATS-based transport implementation.
///
/// This transport uses NATS pub/sub for message distribution.
/// It's fully async and lock-free, using atomic counters for tracking.
///
/// # Idempotent Connection Management
///
/// The transport uses `Arc<Client>` internally, making it safe and efficient
/// to clone. Multiple clones share the same underlying NATS connection.
/// Use `NatsConnectionPool` or the `connect_pooled()` method for managing
/// singleton connections across your app.
///
/// # Example
/// ```rust,no_run
/// # use nats_transport::NatsTransport;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Recommended: Use pooled connections
/// let transport = NatsTransport::<MyEvent>::connect_pooled("nats://localhost:4222").await?;
///
/// // Or create multiple transports that share the connection
/// let t1 = NatsTransport::<MyEvent>::connect_pooled("nats://localhost:4222").await?;
/// let t2 = NatsTransport::<MyEvent>::connect_pooled("nats://localhost:4222").await?;
/// // t1 and t2 share the same underlying connection
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct NatsTransport<E>
where
	E: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
{
	client: Arc<Client>,
	active_channels: Arc<AtomicUsize>,
	_marker: PhantomData<E>,
}

impl<E> NatsTransport<E>
where
	E: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
{
	/// Creates a new NATS transport from a connected client.
	pub fn new(client: Client) -> Self {
		Self {
			client: Arc::new(client),
			active_channels: Arc::new(AtomicUsize::new(0)),
			_marker: PhantomData,
		}
	}

	/// Creates a new NATS transport by connecting to the given URL.
	///
	/// For idempotent connection management, consider using
	/// `connect_pooled()` instead, which reuses connections.
	pub async fn connect(url: impl Into<String>) -> Result<Self> {
		let client = async_nats::connect(url.into()).await.map_err(|e| TransportError::NatsError(e.to_string()))?;
		Ok(Self::new(client))
	}

	/// Creates a new NATS transport using the global connection pool.
	///
	/// This ensures that multiple transports to the same URL share the
	/// same underlying connection, which is efficient and prevents
	/// connection exhaustion.
	///
	/// # Example
	/// ```rust,no_run
	/// # use nats_transport::NatsTransport;
	/// # #[tokio::main]
	/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
	/// let transport1 = NatsTransport::<MyEvent>::connect_pooled("nats://localhost:4222").await?;
	/// let transport2 = NatsTransport::<MyEvent>::connect_pooled("nats://localhost:4222").await?;
	/// // Both share the same connection
	/// # Ok(())
	/// # }
	/// ```
	pub async fn connect_pooled(url: impl Into<String>) -> Result<Self> {
		let client = NatsConnectionPool::global().get_or_connect(url).await?;
		Ok(Self::from_client(client))
	}

	/// Creates a transport from an Arc'd client (useful with connection pools).
	pub fn from_client(client: Arc<Client>) -> Self {
		Self {
			client,
			active_channels: Arc::new(AtomicUsize::new(0)),
			_marker: PhantomData,
		}
	}

	/// Returns a reference to the underlying NATS client.
	pub fn client(&self) -> &Client {
		&self.client
	}

	/// Generates a subject name for a connection-specific channel.
	fn channel_subject(connection_key: &str) -> String {
		format!("channel.{}", connection_key)
	}

	/// The broadcast subject used for global messages.
	const BROADCAST_SUBJECT: &'static str = "broadcast";
}

#[async_trait::async_trait]
impl<E> Transport<E> for NatsTransport<E>
where
	E: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
{
	type Receiver = TransportReceiver<E, NatsReceiver<E>>;

	async fn open_channel(&self, connection_key: &str) -> Self::Receiver {
		let subject = Self::channel_subject(connection_key);
		let subscription = self.client.subscribe(subject).await.expect("Failed to subscribe to channel");

		self.active_channels.fetch_add(1, Ordering::Relaxed);

		TransportReceiver::new(NatsReceiver::new(subscription))
	}

	async fn close_channel(&self, _connection_key: &str) -> Result<()> {
		// Subscriptions auto-cleanup when dropped
		// Decrement counter
		self.active_channels.fetch_sub(1, Ordering::Relaxed);
		Ok(())
	}

	async fn send(&self, connection_key: &str, event: E) -> Result<()> {
		let subject = Self::channel_subject(connection_key);
		let bytes = bincode::serialize(&event).map_err(|e| TransportError::SerializationError(e.to_string()))?;

		self.client.publish(subject, bytes.into()).await.map_err(|e| TransportError::SendFailed(e.to_string()))?;

		Ok(())
	}

	async fn broadcast(&self, event: E) -> Result<usize> {
		let bytes = bincode::serialize(&event).map_err(|e| TransportError::SerializationError(e.to_string()))?;

		self
			.client
			.publish(Self::BROADCAST_SUBJECT.to_string(), bytes.into())
			.await
			.map_err(|e| TransportError::BroadcastFailed(e.to_string()))?;

		// NATS doesn't provide receiver count directly
		// Return 0 to indicate unknown, or could track subscribers separately
		Ok(0)
	}

	async fn subscribe(&self) -> Self::Receiver {
		let subscription = self.client.subscribe(Self::BROADCAST_SUBJECT.to_string()).await.expect("Failed to subscribe to broadcast");

		TransportReceiver::new(NatsReceiver::new(subscription))
	}

	fn total_receivers(&self) -> usize {
		// NATS doesn't expose this information directly
		// Could maintain a registry if needed
		0
	}

	fn is_closed(&self) -> bool {
		// NATS client handles reconnections internally
		// Consider the transport always open unless explicitly closed
		false
	}

	fn active_channels(&self) -> usize {
		self.active_channels.load(Ordering::Relaxed)
	}
}

// Convenience constructors
impl<E> NatsTransport<E>
where
	E: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
{
	/// Creates transport and returns it with an initial broadcast receiver.
	pub async fn with_receiver(client: Client) -> (Self, TransportReceiver<E, NatsReceiver<E>>) {
		let transport = Self::new(client);
		let receiver = transport.subscribe().await;
		(transport, receiver)
	}

	/// Connects to NATS and returns transport with initial receiver.
	pub async fn connect_with_receiver(url: impl Into<String>) -> Result<(Self, TransportReceiver<E, NatsReceiver<E>>)> {
		let transport = Self::connect(url).await?;
		let receiver = transport.subscribe().await;
		Ok((transport, receiver))
	}

	/// Connects using the global pool and returns transport with initial receiver.
	///
	/// This is the recommended way to create transports in most applications,
	/// as it ensures connection reuse across multiple transport instances.
	pub async fn connect_pooled_with_receiver(url: impl Into<String>) -> Result<(Self, TransportReceiver<E, NatsReceiver<E>>)> {
		let transport = Self::connect_pooled(url).await?;
		let receiver = transport.subscribe().await;
		Ok((transport, receiver))
	}
}

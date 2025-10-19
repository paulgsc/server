#![cfg(feature = "nats")]

use super::pool::NatsConnectionPool;
use super::receiver::NatsReceiver;
use crate::error::{Result, TransportError};
use crate::receiver::TransportReceiver;
use crate::traits::Transport;
use async_nats::Client;
use bincode::{Decode, Encode};
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
/// # use some_transport::NatsTransport;
/// # use bincode::{Encode, Decode};
/// # #[derive(Clone, Debug, PartialEq, Encode, Decode)]
/// # pub struct MyEvent {};
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
	E: Clone + Send + Sync + Encode + Decode<()> + 'static,
{
	client: Arc<Client>,
	active_channels: Arc<AtomicUsize>,
	_marker: PhantomData<E>,
}

impl<E> NatsTransport<E>
where
	E: Clone + Send + Sync + Encode + Decode<()> + 'static,
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
	/// # use some_transport::NatsTransport;
	/// # use bincode::{Encode, Decode};
	/// # #[derive(Clone, Debug, PartialEq, Encode, Decode)]
	/// # pub struct MyEvent {};
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
	E: Clone + Send + Sync + Encode + Decode<()> + 'static,
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
		let bytes = bincode::encode_to_vec(&event, bincode::config::standard()).map_err(|e| TransportError::SerializationError(e.to_string()))?;

		self.client.publish(subject, bytes.into()).await.map_err(|e| TransportError::SendFailed(e.to_string()))?;

		Ok(())
	}

	async fn broadcast(&self, event: E) -> Result<usize> {
		let bytes = bincode::encode_to_vec(&event, bincode::config::standard()).map_err(|e| TransportError::SerializationError(e.to_string()))?;

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
	E: Clone + Send + Sync + Encode + Decode<()> + 'static,
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

#[cfg(test)]
mod tests {
	use super::*;
	use bincode::{Decode, Encode};
	use std::time::Duration;
	use tokio::time::timeout;

	// Test event type
	#[derive(Clone, Debug, PartialEq, Encode, Decode)]
	struct TestEvent {
		id: u64,
		message: String,
	}

	// Helper to get test NATS URL from environment or use default
	fn nats_url() -> String {
		std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string())
	}

	// Helper to check if NATS is available
	async fn nats_available() -> bool {
		async_nats::connect(nats_url()).await.is_ok()
	}

	#[tokio::test]
	async fn test_new_transport() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let client = async_nats::connect(nats_url()).await.unwrap();
		let transport = NatsTransport::<TestEvent>::new(client);

		assert_eq!(transport.active_channels(), 0);
		assert!(!transport.is_closed());
	}

	#[tokio::test]
	async fn test_connect() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let result = NatsTransport::<TestEvent>::connect(nats_url()).await;
		assert!(result.is_ok());

		let transport = result.unwrap();
		assert_eq!(transport.active_channels(), 0);
	}

	#[tokio::test]
	async fn test_connect_pooled() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let t1 = NatsTransport::<TestEvent>::connect_pooled(nats_url()).await.unwrap();
		let t2 = NatsTransport::<TestEvent>::connect_pooled(nats_url()).await.unwrap();

		// Both should be created successfully
		assert_eq!(t1.active_channels(), 0);
		assert_eq!(t2.active_channels(), 0);

		// They should share the same underlying connection
		// (verified by pointer equality of Arc<Client>)
		assert!(Arc::ptr_eq(&t1.client, &t2.client));
	}

	#[tokio::test]
	async fn test_from_client() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let client = async_nats::connect(nats_url()).await.unwrap();
		let arc_client = Arc::new(client);
		let transport = NatsTransport::<TestEvent>::from_client(arc_client.clone());

		assert!(Arc::ptr_eq(&transport.client, &arc_client));
	}

	#[tokio::test]
	async fn test_open_and_close_channel() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		assert_eq!(transport.active_channels(), 0);

		let _receiver = transport.open_channel("test-connection").await;
		assert_eq!(transport.active_channels(), 1);

		transport.close_channel("test-connection").await.unwrap();
		assert_eq!(transport.active_channels(), 0);
	}

	#[tokio::test]
	async fn test_multiple_channels() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();

		let _r1 = transport.open_channel("conn1").await;
		let _r2 = transport.open_channel("conn2").await;
		let _r3 = transport.open_channel("conn3").await;

		assert_eq!(transport.active_channels(), 3);

		transport.close_channel("conn1").await.unwrap();
		assert_eq!(transport.active_channels(), 2);

		transport.close_channel("conn2").await.unwrap();
		assert_eq!(transport.active_channels(), 1);
	}

	#[tokio::test]
	async fn test_send_and_receive() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		let mut receiver = transport.open_channel("test-send").await;

		let event = TestEvent {
			id: 42,
			message: "Hello NATS".to_string(),
		};

		transport.send("test-send", event.clone()).await.unwrap();

		let received = timeout(Duration::from_secs(2), receiver.recv())
			.await
			.expect("Timeout waiting for message")
			.expect("Failed to receive message");

		assert_eq!(received, event);
	}

	#[tokio::test]
	async fn test_send_multiple_messages() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		let mut receiver = transport.open_channel("test-multi").await;

		let events = vec![
			TestEvent {
				id: 1,
				message: "First".to_string(),
			},
			TestEvent {
				id: 2,
				message: "Second".to_string(),
			},
			TestEvent {
				id: 3,
				message: "Third".to_string(),
			},
		];

		for event in &events {
			transport.send("test-multi", event.clone()).await.unwrap();
		}

		for expected in &events {
			let received = timeout(Duration::from_secs(2), receiver.recv()).await.expect("Timeout").expect("Failed to receive");
			assert_eq!(&received, expected);
		}
	}

	#[tokio::test]
	async fn test_broadcast() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		let mut r1 = transport.subscribe().await;
		let mut r2 = transport.subscribe().await;

		let event = TestEvent {
			id: 99,
			message: "Broadcast message".to_string(),
		};

		transport.broadcast(event.clone()).await.unwrap();

		// Both receivers should get the message
		let received1 = timeout(Duration::from_secs(2), r1.recv()).await.expect("Timeout on r1").expect("Failed to receive on r1");
		assert_eq!(received1, event);

		let received2 = timeout(Duration::from_secs(2), r2.recv()).await.expect("Timeout on r2").expect("Failed to receive on r2");
		assert_eq!(received2, event);
	}

	#[tokio::test]
	async fn test_channel_isolation() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		let mut r1 = transport.open_channel("channel1").await;
		let mut r2 = transport.open_channel("channel2").await;

		let event1 = TestEvent {
			id: 1,
			message: "Channel 1".to_string(),
		};
		let event2 = TestEvent {
			id: 2,
			message: "Channel 2".to_string(),
		};

		transport.send("channel1", event1.clone()).await.unwrap();
		transport.send("channel2", event2.clone()).await.unwrap();

		// Each receiver should only get its own channel's message
		let received1 = timeout(Duration::from_secs(2), r1.recv()).await.expect("Timeout on r1").expect("Failed to receive on r1");
		assert_eq!(received1, event1);

		let received2 = timeout(Duration::from_secs(2), r2.recv()).await.expect("Timeout on r2").expect("Failed to receive on r2");
		assert_eq!(received2, event2);
	}

	#[tokio::test]
	async fn test_subscribe() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		let mut receiver = transport.subscribe().await;

		let event = TestEvent {
			id: 123,
			message: "Subscribe test".to_string(),
		};

		transport.broadcast(event.clone()).await.unwrap();

		let received = timeout(Duration::from_secs(2), receiver.recv()).await.expect("Timeout").expect("Failed to receive");
		assert_eq!(received, event);
	}

	#[tokio::test]
	async fn test_clone_transport() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let t1 = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		let t2 = t1.clone();

		// Both should share the same client
		assert!(Arc::ptr_eq(&t1.client, &t2.client));

		// Both should work independently
		let mut r1 = t1.open_channel("clone-test-1").await;
		let mut r2 = t2.open_channel("clone-test-2").await;

		let event1 = TestEvent {
			id: 1,
			message: "From t1".to_string(),
		};
		let event2 = TestEvent {
			id: 2,
			message: "From t2".to_string(),
		};

		t1.send("clone-test-1", event1.clone()).await.unwrap();
		t2.send("clone-test-2", event2.clone()).await.unwrap();

		let received1 = timeout(Duration::from_secs(2), r1.recv()).await.unwrap().unwrap();
		let received2 = timeout(Duration::from_secs(2), r2.recv()).await.unwrap().unwrap();

		assert_eq!(received1, event1);
		assert_eq!(received2, event2);
	}

	#[tokio::test]
	async fn test_with_receiver() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let client = async_nats::connect(nats_url()).await.unwrap();
		let (transport, mut receiver) = NatsTransport::<TestEvent>::with_receiver(client).await;

		let event = TestEvent {
			id: 777,
			message: "With receiver".to_string(),
		};

		transport.broadcast(event.clone()).await.unwrap();

		let received = timeout(Duration::from_secs(2), receiver.recv()).await.expect("Timeout").expect("Failed to receive");
		assert_eq!(received, event);
	}

	#[tokio::test]
	async fn test_connect_with_receiver() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let result = NatsTransport::<TestEvent>::connect_with_receiver(nats_url()).await;
		assert!(result.is_ok());

		let (transport, mut receiver) = result.unwrap();

		let event = TestEvent {
			id: 888,
			message: "Connect with receiver".to_string(),
		};

		transport.broadcast(event.clone()).await.unwrap();

		let received = timeout(Duration::from_secs(2), receiver.recv()).await.expect("Timeout").expect("Failed to receive");
		assert_eq!(received, event);
	}

	#[tokio::test]
	async fn test_connect_pooled_with_receiver() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let result = NatsTransport::<TestEvent>::connect_pooled_with_receiver(nats_url()).await;
		assert!(result.is_ok());

		let (transport, mut receiver) = result.unwrap();

		let event = TestEvent {
			id: 999,
			message: "Pooled with receiver".to_string(),
		};

		transport.broadcast(event.clone()).await.unwrap();

		let received = timeout(Duration::from_secs(2), receiver.recv()).await.expect("Timeout").expect("Failed to receive");
		assert_eq!(received, event);
	}

	#[tokio::test]
	async fn test_client_getter() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		let client = transport.client();

		// Verify we can use the client directly
		let result = client.publish("test.subject".to_string(), "test".into()).await;
		assert!(result.is_ok());
	}

	#[tokio::test]
	async fn test_total_receivers() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();

		// NATS doesn't expose this, so it should return 0
		assert_eq!(transport.total_receivers(), 0);
	}

	#[tokio::test]
	async fn test_is_closed() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();

		// Transport should never report as closed (NATS handles reconnections)
		assert!(!transport.is_closed());
	}

	#[tokio::test]
	async fn test_channel_subject_format() {
		let subject = NatsTransport::<TestEvent>::channel_subject("my-connection");
		assert_eq!(subject, "channel.my-connection");
	}

	#[tokio::test]
	async fn test_broadcast_subject() {
		assert_eq!(NatsTransport::<TestEvent>::BROADCAST_SUBJECT, "broadcast");
	}

	#[tokio::test]
	async fn test_concurrent_sends() {
		if !nats_available().await {
			println!("Skipping test: NATS not available");
			return;
		}

		let transport = NatsTransport::<TestEvent>::connect(nats_url()).await.unwrap();
		let mut receiver = transport.open_channel("concurrent-test").await;

		let handles: Vec<_> = (0..10)
			.map(|i| {
				let t = transport.clone();
				tokio::spawn(async move {
					let event = TestEvent {
						id: i,
						message: format!("Message {}", i),
					};
					t.send("concurrent-test", event).await
				})
			})
			.collect();

		// Wait for all sends to complete
		for handle in handles {
			assert!(handle.await.unwrap().is_ok());
		}

		// Receive all messages
		let mut received_ids = Vec::new();
		for _ in 0..10 {
			let msg = timeout(Duration::from_secs(2), receiver.recv()).await.expect("Timeout").expect("Failed to receive");
			received_ids.push(msg.id);
		}

		received_ids.sort();
		assert_eq!(received_ids, (0..10).collect::<Vec<_>>());
	}

	#[tokio::test]
	async fn test_error_invalid_url() {
		let result = NatsTransport::<TestEvent>::connect("invalid://url:99999").await;
		assert!(result.is_err());
	}
}

#![cfg(feature = "inmem")]

use super::receiver::InMemReceiver; // ← Import local implementation
use crate::error::{Result, TransportError};
use crate::receiver::TransportReceiver; // ← Import from shared core
use crate::traits::Transport;
use async_broadcast::{broadcast, Sender};
use dashmap::DashMap;
use std::sync::Arc;

/// In-memory transport implementation using async_broadcast.
///
/// This transport provides high-performance, in-process message delivery
/// using lock-free broadcast channels. It's ideal for:
///
/// - Testing transport-based systems
/// - Single-process applications
/// - Local pubsub within a process
/// - Development environments
///
/// # Architecture
///
/// - **Main channel**: Global broadcast to all subscribers
/// - **Connection channels**: Isolated channels per connection key
/// - **Lock-free**: Uses `DashMap` and `async_broadcast` for concurrency
///
/// # Example
///
/// ```rust,no_run
/// use transport::inmem::InMemTransport;
/// use transport::traits::Transport;
///
/// #[tokio::main]
/// async fn main() {
///     // Create with buffer size of 100 messages
///     let transport = InMemTransport::<String>::new(100);
///     
///     // Subscribe to global broadcasts
///     let mut rx = transport.subscribe();
///     
///     // Broadcast to all subscribers
///     transport.broadcast("Hello world!".to_string()).await.ok();
///     
///     // Receive the broadcast
///     let msg = rx.recv().await.unwrap();
///     println!("Received: {}", msg);
/// }
/// ```
#[derive(Clone)]
pub struct InMemTransport<E>
where
	E: Clone + Send + Sync + 'static,
{
	main_sender: Sender<E>,
	_keep_alive: async_broadcast::Receiver<E>, // Keep channel open
	connection_channels: Arc<DashMap<String, Sender<E>>>,
}

impl<E> InMemTransport<E>
where
	E: Clone + Send + Sync + 'static,
{
	/// Creates a new in-memory transport layer.
	///
	/// # Arguments
	///
	/// * `buffer_size` - Maximum number of messages to buffer per channel.
	///   When overflow occurs, older messages are dropped.
	///
	/// # Example
	///
	/// ```rust,no_run
	/// use transport::inmem::InMemTransport;
	///
	/// let transport = InMemTransport::<String>::new(100);
	/// ```
	#[must_use]
	pub fn new(buffer_size: usize) -> Self {
		let (mut main_sender, keep_alive) = broadcast::<E>(buffer_size);
		main_sender.set_await_active(false);
		main_sender.set_overflow(true);

		Self {
			main_sender,
			_keep_alive: keep_alive,
			connection_channels: Arc::new(DashMap::new()),
		}
	}

	/// Returns the underlying sender (for diagnostics only).
	#[must_use]
	pub fn main_sender(&self) -> &Sender<E> {
		&self.main_sender
	}
}

#[async_trait::async_trait]
impl<E> Transport<E> for InMemTransport<E>
where
	E: Clone + Send + Sync + 'static,
{
	type Receiver = TransportReceiver<E, InMemReceiver<E>>;

	async fn open_channel(&self, connection_key: &str) -> Self::Receiver {
		let (mut sender, receiver) = broadcast::<E>(100);
		sender.set_await_active(false);
		sender.set_overflow(true);
		self.connection_channels.insert(connection_key.to_string(), sender);

		TransportReceiver::new(InMemReceiver::new(receiver))
	}

	async fn close_channel(&self, connection_key: &str) -> Result<()> {
		self.connection_channels.remove(connection_key);
		Ok(())
	}

	async fn send(&self, connection_key: &str, event: E) -> Result<()> {
		if let Some(sender) = self.connection_channels.get(connection_key) {
			sender.broadcast(event).await.map(|_| ()).map_err(|e| TransportError::SendFailed(e.to_string()))
		} else {
			Err(TransportError::ConnectionNotFound(connection_key.to_string()))
		}
	}

	async fn broadcast(&self, event: E) -> Result<usize> {
		self
			.main_sender
			.broadcast(event)
			.await
			.map(|res| res.is_some() as usize)
			.map_err(|e| TransportError::BroadcastFailed(e.to_string()))
	}

	async fn subscribe(&self) -> TransportReceiver<E, InMemReceiver<E>> {
		let receiver = self.main_sender.new_receiver();
		TransportReceiver::new(InMemReceiver::new(receiver))
	}

	fn total_receivers(&self) -> usize {
		self.main_sender.receiver_count()
	}

	fn is_closed(&self) -> bool {
		self.main_sender.is_closed()
	}

	fn active_channels(&self) -> usize {
		self.connection_channels.len()
	}
}

// === Convenience constructors ===

impl<E> InMemTransport<E>
where
	E: Clone + Send + Sync + 'static,
{
	/// Creates transport and returns an initial receiver (for convenience).
	///
	/// This is useful when you want to start listening to broadcasts immediately.
	///
	/// # Example
	///
	/// ```rust,no_run
	/// use transport::inmem::InMemTransport;
	/// use transport::traits::Transport;
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (transport, mut rx) = InMemTransport::<String>::with_receiver(100);
	///     
	///     // Can immediately start receiving
	///     tokio::spawn(async move {
	///         while let Ok(msg) = rx.recv().await {
	///             println!("Got: {}", msg);
	///         }
	///     });
	///     
	///     transport.broadcast("Hello!".to_string()).await.ok();
	/// }
	/// ```
	#[must_use]
	pub async fn with_receiver(buffer_size: usize) -> (Self, TransportReceiver<E, InMemReceiver<E>>) {
		let transport = Self::new(buffer_size);
		let receiver = transport.subscribe().await;
		(transport, receiver)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn test_broadcast() {
		let (transport, mut rx) = InMemTransport::<String>::with_receiver(10);

		transport.broadcast("test message".to_string()).await.unwrap();

		let msg = rx.recv().await.unwrap();
		assert_eq!(msg, "test message");
	}

	#[tokio::test]
	async fn test_multiple_subscribers() {
		let (transport, mut rx1) = InMemTransport::<i32>::with_receiver(10);
		let mut rx2 = transport.subscribe().await;
		let mut rx3 = transport.subscribe().await;

		transport.broadcast(42).await.unwrap();

		assert_eq!(rx1.recv().await.unwrap(), 42);
		assert_eq!(rx2.recv().await.unwrap(), 42);
		assert_eq!(rx3.recv().await.unwrap(), 42);
	}

	#[tokio::test]
	async fn test_channel_send() {
		let transport = InMemTransport::<String>::new(10);
		let mut rx = transport.open_channel("test_conn").await;

		transport.send("test_conn", "channel message".to_string()).await.unwrap();

		let msg = rx.recv().await.unwrap();
		assert_eq!(msg, "channel message");
	}

	#[tokio::test]
	async fn test_channel_not_found() {
		let transport = InMemTransport::<String>::new(10);

		let result = transport.send("nonexistent", "message".to_string()).await;

		assert!(result.is_err());
		assert!(matches!(result.unwrap_err(), TransportError::ConnectionNotFound(_)));
	}

	#[tokio::test]
	async fn test_close_channel() {
		let transport = InMemTransport::<String>::new(10);
		let _rx = transport.open_channel("test_conn").await;

		assert_eq!(transport.active_channels(), 1);

		transport.close_channel("test_conn").await.unwrap();

		assert_eq!(transport.active_channels(), 0);
	}

	#[tokio::test]
	async fn test_total_receivers() {
		let (transport, _rx1) = InMemTransport::<String>::with_receiver(10);
		assert_eq!(transport.total_receivers(), 1);

		let _rx2 = transport.subscribe().await;
		assert_eq!(transport.total_receivers(), 2);

		let _rx3 = transport.subscribe().await;
		assert_eq!(transport.total_receivers(), 3);
	}

	#[tokio::test]
	async fn test_is_closed() {
		let transport = InMemTransport::<String>::new(10);
		assert!(!transport.is_closed());
	}

	#[tokio::test]
	async fn test_active_channels() {
		let transport = InMemTransport::<String>::new(10);
		assert_eq!(transport.active_channels(), 0);

		let _rx1 = transport.open_channel("conn1").await;
		assert_eq!(transport.active_channels(), 1);

		let _rx2 = transport.open_channel("conn2").await;
		assert_eq!(transport.active_channels(), 2);

		transport.close_channel("conn1").await.ok();
		assert_eq!(transport.active_channels(), 1);
	}
}

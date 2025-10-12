use crate::error::{Result, TransportError};
use crate::receiver::{InMemReceiver, TransportReceiver};
use crate::traits::Transport;
use async_broadcast::{broadcast, Sender};
use dashmap::DashMap;
use std::sync::Arc;

/// In-memory transport implementation using `async_broadcast`.
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

		TransportReceiver::new(InMemReceiver(receiver))
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

	fn subscribe(&self) -> TransportReceiver<E, InMemReceiver<E>> {
		let receiver = self.main_sender.new_receiver();
		TransportReceiver::new(InMemReceiver(receiver))
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

// === Convenience constructor ===

impl<E> InMemTransport<E>
where
	E: Clone + Send + Sync + 'static,
{
	/// Creates transport and returns an initial receiver (for convenience).
	#[must_use]
	pub fn with_receiver(buffer_size: usize) -> (Self, TransportReceiver<E, InMemReceiver<E>>) {
		let transport = Self::new(buffer_size);
		let receiver = transport.subscribe();
		(transport, receiver)
	}
}

#![cfg(feature = "inmem")]

use crate::error::{Result, TransportError};
use crate::receiver::ReceiverTrait;
use async_broadcast::{Receiver, RecvError, TryRecvError};
use async_trait::async_trait;

/// In-memory receiver implementation using `async_broadcast`.
///
/// This receiver wraps an `async_broadcast::Receiver` and implements
/// the generic `ReceiverTrait` interface, allowing it to work seamlessly
/// with the transport-agnostic `TransportReceiver` wrapper.
///
/// # Example
/// ```rust,no_run
/// use async_broadcast::broadcast;
/// use transport::inmem::InMemReceiver;
/// use transport::receiver::{TransportReceiver, ReceiverTrait};
///
/// let (tx, rx) = broadcast::<String>(10);
/// let receiver = InMemReceiver(rx);
/// let mut transport_rx = TransportReceiver::new(receiver);
///
/// // Now you can use it like any other transport receiver
/// // let msg = transport_rx.recv().await?;
/// ```
#[derive(Clone)]
pub struct InMemReceiver<E>(pub Receiver<E>);

impl<E> InMemReceiver<E> {
	/// Creates a new in-memory receiver from an `async_broadcast::Receiver`.
	#[inline]
	pub fn new(receiver: Receiver<E>) -> Self {
		Self(receiver)
	}

	/// Returns a reference to the underlying receiver.
	#[inline]
	pub fn inner(&self) -> &Receiver<E> {
		&self.0
	}

	/// Returns a mutable reference to the underlying receiver.
	#[inline]
	pub fn inner_mut(&mut self) -> &mut Receiver<E> {
		&mut self.0
	}

	/// Consumes the wrapper and returns the underlying receiver.
	#[inline]
	pub fn into_inner(self) -> Receiver<E> {
		self.0
	}
}

#[async_trait]
impl<E> ReceiverTrait<E> for InMemReceiver<E>
where
	E: Clone + Send + Sync + 'static,
{
	async fn recv(&mut self) -> Result<E> {
		match self.0.recv().await {
			Ok(event) => Ok(event),
			Err(RecvError::Closed) => Err(TransportError::Closed),
			Err(RecvError::Overflowed(n)) => Err(TransportError::Overflowed(n)),
		}
	}

	fn try_recv(&mut self) -> Result<E> {
		match self.0.try_recv() {
			Ok(event) => Ok(event),
			Err(TryRecvError::Closed) => Err(TransportError::Closed),
			Err(TryRecvError::Overflowed(n)) => Err(TransportError::Overflowed(n)),
			Err(TryRecvError::Empty) => Err(TransportError::Other("Channel empty".into())),
		}
	}
}

// Implement From for ergonomic conversions
impl<E> From<Receiver<E>> for InMemReceiver<E> {
	fn from(receiver: Receiver<E>) -> Self {
		Self::new(receiver)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::receiver::TransportReceiver;
	use async_broadcast::broadcast;

	#[tokio::test]
	async fn test_inmem_receiver_recv() {
		let (mut tx, rx) = broadcast::<String>(10);
		let receiver = InMemReceiver::new(rx);
		let mut transport_rx = TransportReceiver::new(receiver);

		tx.broadcast("test message".to_string()).await.ok();

		let result = transport_rx.recv().await;
		assert!(result.is_ok());
		assert_eq!(result.unwrap(), "test message");
	}

	#[tokio::test]
	async fn test_inmem_receiver_try_recv() {
		let (mut tx, rx) = broadcast::<i32>(10);
		let receiver = InMemReceiver::new(rx);
		let mut transport_rx = TransportReceiver::new(receiver);

		tx.broadcast(42).await.ok();

		// Small delay to ensure message is delivered
		tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

		let result = transport_rx.try_recv();
		assert!(result.is_ok());
		assert_eq!(result.unwrap(), 42);
	}

	#[tokio::test]
	async fn test_inmem_receiver_closed() {
		let (tx, rx) = broadcast::<String>(10);
		let receiver = InMemReceiver::new(rx);
		let mut transport_rx = TransportReceiver::new(receiver);

		// Drop sender to close channel
		drop(tx);

		let result = transport_rx.recv().await;
		assert!(result.is_err());
		assert!(matches!(result.unwrap_err(), TransportError::Closed));
	}

	#[tokio::test]
	async fn test_from_conversion() {
		let (_tx, rx) = broadcast::<String>(10);
		let _receiver: InMemReceiver<String> = rx.into();
	}
}

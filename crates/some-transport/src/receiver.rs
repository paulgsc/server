#![cfg(feature = "inmem")]

use crate::error::Result;
use async_trait::async_trait;
use std::marker::PhantomData;

/// Public, transport-agnostic receiver wrapper.
///
/// This struct provides a unified interface around various message receivers
/// (in-memory, NATS, Redis streams, etc.), as long as they implement
/// [`ReceiverTrait`].
pub struct TransportReceiver<E, R>
where
	E: Clone + Send + Sync + 'static,
	R: ReceiverTrait<E> + Send + 'static,
{
	inner: R,
	_marker: PhantomData<E>,
}

impl<E, R> TransportReceiver<E, R>
where
	E: Clone + Send + Sync + 'static,
	R: ReceiverTrait<E> + Send + 'static,
{
	/// Creates a new `TransportReceiver` from a type implementing [`ReceiverTrait`].
	pub fn new(receiver: R) -> Self {
		Self {
			inner: receiver,
			_marker: PhantomData,
		}
	}

	/// Receives a message asynchronously, waiting until one is available.
	pub async fn recv(&mut self) -> Result<E> {
		self.inner.recv().await
	}

	/// Attempts to receive a message without blocking.
	pub fn try_recv(&mut self) -> Result<E> {
		self.inner.try_recv()
	}
}

/// Trait representing a generic message receiver.
///
/// Each transport implementation (e.g. NATS, in-memory, etc.)
/// must provide an implementation of this trait.
///
/// Implementors should define how messages are received asynchronously
/// (`recv`) and non-blockingly (`try_recv`).
#[async_trait]
pub trait ReceiverTrait<E>: Send
where
	E: Clone + Send + Sync + 'static,
{
	/// Waits for and receives the next message.
	async fn recv(&mut self) -> Result<E>;

	/// Attempts to receive a message immediately.
	fn try_recv(&mut self) -> Result<E>;
}

#[cfg(feature = "inmem")]
mod inmem_impl {
	use super::*;
	use crate::error::TransportError;
	use async_broadcast::{Receiver, RecvError, TryRecvError};

	/// In-memory receiver implementation using `async_broadcast`.
	#[derive(Clone)]
	pub struct InMemReceiver<E>(pub Receiver<E>);

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
}

#[cfg(feature = "inmem")]
pub use inmem_impl::InMemReceiver;

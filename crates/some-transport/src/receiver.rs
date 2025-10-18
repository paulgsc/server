use crate::error::Result;
use async_trait::async_trait;
use std::marker::PhantomData;

/// Public, transport-agnostic receiver wrapper.
///
/// This struct provides a unified interface around various message receivers
/// (in-memory, NATS, Redis streams, etc.), as long as they implement
/// [`ReceiverTrait`].
///
/// # Example
/// ```rust,no_run
/// use transport::receiver::{TransportReceiver, ReceiverTrait};
///
/// async fn handle_messages<R>(mut rx: TransportReceiver<MyEvent, R>)
/// where
///     R: ReceiverTrait<MyEvent> + Send + 'static,
/// {
///     while let Ok(event) = rx.recv().await {
///         println!("Received: {:?}", event);
///     }
/// }
/// ```
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
	#[inline]
	pub fn new(receiver: R) -> Self {
		Self {
			inner: receiver,
			_marker: PhantomData,
		}
	}

	/// Receives a message asynchronously, waiting until one is available.
	#[inline]
	pub async fn recv(&mut self) -> Result<E> {
		self.inner.recv().await
	}

	/// Attempts to receive a message without blocking.
	#[inline]
	pub fn try_recv(&mut self) -> Result<E> {
		self.inner.try_recv()
	}

	/// Consumes the wrapper and returns the inner receiver.
	#[inline]
	pub fn into_inner(self) -> R {
		self.inner
	}

	/// Returns a reference to the inner receiver.
	#[inline]
	pub fn inner(&self) -> &R {
		&self.inner
	}

	/// Returns a mutable reference to the inner receiver.
	#[inline]
	pub fn inner_mut(&mut self) -> &mut R {
		&mut self.inner
	}
}

/// Trait representing a generic message receiver.
///
/// Each transport implementation (e.g. NATS, in-memory, etc.)
/// must provide an implementation of this trait.
///
/// Implementors should define how messages are received asynchronously
/// (`recv`) and non-blockingly (`try_recv`).
///
/// # Example Implementation
/// ```rust,no_run
/// use async_trait::async_trait;
/// use transport::receiver::ReceiverTrait;
/// use transport::error::Result;
///
/// struct MyReceiver {
///     // ... implementation details
/// }
///
/// #[async_trait]
/// impl ReceiverTrait<MyEvent> for MyReceiver {
///     async fn recv(&mut self) -> Result<MyEvent> {
///         // Wait for next message
///         todo!()
///     }
///
///     fn try_recv(&mut self) -> Result<MyEvent> {
///         // Try to receive without blocking
///         todo!()
///     }
/// }
/// ```
#[async_trait]
pub trait ReceiverTrait<E>: Send
where
	E: Clone + Send + Sync + 'static,
{
	/// Waits for and receives the next message.
	///
	/// This is an async method that will await until a message is available
	/// or the channel is closed.
	async fn recv(&mut self) -> Result<E>;

	/// Attempts to receive a message immediately.
	///
	/// Returns immediately with either a message or an error indicating
	/// the channel is empty, closed, or overflowed.
	fn try_recv(&mut self) -> Result<E>;
}

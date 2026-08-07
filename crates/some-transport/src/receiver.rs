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
/// use some_transport::{TransportReceiver, ReceiverTrait};
/// # #[derive(Clone, Debug, PartialEq)]
/// # pub struct MyEvent {}
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

#[derive(Clone)]
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
	pub const fn new(receiver: R) -> Self {
		Self {
			inner: receiver,
			_marker: PhantomData,
		}
	}

	/// Receives a message asynchronously, waiting until one is available.
	///
	/// # Errors
	///
	/// Propagates whatever the underlying receiver reports — typically
	/// [`TransportError::Closed`] once the channel has been closed, or
	/// [`TransportError::Overflowed`] if this receiver lagged far enough
	/// behind that messages were dropped.
	///
	/// [`TransportError::Closed`]: crate::error::TransportError::Closed
	/// [`TransportError::Overflowed`]: crate::error::TransportError::Overflowed
	#[inline]
	pub async fn recv(&mut self) -> Result<E> {
		self.inner.recv().await
	}

	/// Attempts to receive a message without blocking.
	///
	/// # Errors
	///
	/// Propagates whatever the underlying receiver reports — typically
	/// [`TransportError::Closed`] when the channel is closed, or
	/// [`TransportError::Overflowed`] when messages were dropped. An empty
	/// channel is also reported as an error rather than blocking.
	///
	/// [`TransportError::Closed`]: crate::error::TransportError::Closed
	/// [`TransportError::Overflowed`]: crate::error::TransportError::Overflowed
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
	pub const fn inner(&self) -> &R {
		&self.inner
	}

	/// Returns a mutable reference to the inner receiver.
	#[inline]
	pub const fn inner_mut(&mut self) -> &mut R {
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
/// use some_transport::receiver::ReceiverTrait;
/// use some_transport::error::Result;
///
/// # #[derive(Clone, Debug, PartialEq)]
/// # pub struct MyEvent {}
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
	///
	/// # Errors
	///
	/// Implementors should report [`TransportError::Closed`] once the
	/// channel is closed and [`TransportError::Overflowed`] when messages
	/// were dropped. An empty channel must also be reported as an error
	/// rather than blocking.
	///
	/// [`TransportError::Closed`]: crate::error::TransportError::Closed
	/// [`TransportError::Overflowed`]: crate::error::TransportError::Overflowed
	fn try_recv(&mut self) -> Result<E>;
}

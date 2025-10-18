#![cfg(feature = "nats")]

use crate::error::{Result, TransportError};
use crate::receiver::ReceiverTrait;
use async_nats::Subscriber;
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::marker::PhantomData;

/// NATS receiver implementation.
///
/// This receiver wraps an `async_nats::Subscriber` and implements
/// the generic `ReceiverTrait` interface, allowing it to work seamlessly
/// with the transport-agnostic `TransportReceiver` wrapper.
///
/// Messages are automatically serialized/deserialized using bincode.
///
/// # Example
/// ```rust,no_run
/// use transport::nats::NatsReceiver;
/// use transport::receiver::TransportReceiver;
///
/// async fn example(client: async_nats::Client) {
///     let sub = client.subscribe("my.subject").await.unwrap();
///     let receiver = NatsReceiver::<MyEvent>::new(sub);
///     let mut transport_rx = TransportReceiver::new(receiver);
///
///     // Now you can use it like any other transport receiver
///     // let msg = transport_rx.recv().await?;
/// }
/// ```
pub struct NatsReceiver<E>
where
	E: Clone + Send + Sync + 'static,
{
	subscription: Subscriber,
	_marker: PhantomData<E>,
}

impl<E> NatsReceiver<E>
where
	E: Clone + Send + Sync + 'static,
{
	/// Creates a new NATS receiver from an `async_nats::Subscriber`.
	#[inline]
	pub fn new(subscription: Subscriber) -> Self {
		Self {
			subscription,
			_marker: PhantomData,
		}
	}

	/// Returns a reference to the underlying subscription.
	#[inline]
	pub fn inner(&self) -> &Subscriber {
		&self.subscription
	}

	/// Returns a mutable reference to the underlying subscription.
	#[inline]
	pub fn inner_mut(&mut self) -> &mut Subscriber {
		&mut self.subscription
	}

	/// Consumes the wrapper and returns the underlying subscription.
	#[inline]
	pub fn into_inner(self) -> Subscriber {
		self.subscription
	}
}

#[async_trait]
impl<E> ReceiverTrait<E> for NatsReceiver<E>
where
	E: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
{
	async fn recv(&mut self) -> Result<E> {
		match self.subscription.next().await {
			Some(msg) => bincode::deserialize::<E>(&msg.payload).map_err(|e| TransportError::DeserializationError(e.to_string())),
			None => Err(TransportError::Closed),
		}
	}

	fn try_recv(&mut self) -> Result<E> {
		match self.subscription.try_next() {
			Ok(Some(msg)) => bincode::deserialize::<E>(&msg.payload).map_err(|e| TransportError::DeserializationError(e.to_string())),
			Ok(None) => Err(TransportError::Other("Channel empty".into())),
			Err(_) => Err(TransportError::Closed),
		}
	}
}

// Implement From for ergonomic conversions
impl<E> From<Subscriber> for NatsReceiver<E>
where
	E: Clone + Send + Sync + 'static,
{
	fn from(subscription: Subscriber) -> Self {
		Self::new(subscription)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::receiver::TransportReceiver;
	use serde::{Deserialize, Serialize};

	#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
	struct TestEvent {
		id: u32,
		message: String,
	}

	#[tokio::test]
	async fn test_nats_receiver_recv() {
		// This test requires a running NATS server
		if let Ok(client) = async_nats::connect("nats://localhost:4222").await {
			let sub = client.subscribe("test.subject").await.unwrap();
			let receiver = NatsReceiver::<TestEvent>::new(sub);
			let mut transport_rx = TransportReceiver::new(receiver);

			let test_event = TestEvent {
				id: 1,
				message: "test".to_string(),
			};

			let bytes = bincode::serialize(&test_event).unwrap();
			client.publish("test.subject", bytes.into()).await.ok();

			// Give it time to arrive
			tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

			if let Ok(event) = transport_rx.try_recv() {
				assert_eq!(event, test_event);
			}
		}
	}

	#[tokio::test]
	async fn test_nats_receiver_deserialization_error() {
		if let Ok(client) = async_nats::connect("nats://localhost:4222").await {
			let sub = client.subscribe("test.subject.bad").await.unwrap();
			let receiver = NatsReceiver::<TestEvent>::new(sub);
			let mut transport_rx = TransportReceiver::new(receiver);

			// Send invalid data
			client.publish("test.subject.bad", "invalid data".into()).await.ok();

			tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

			let result = transport_rx.try_recv();
			assert!(result.is_err());
			if let Err(TransportError::DeserializationError(_)) = result {
				// Expected error type
			} else {
				panic!("Expected DeserializationError");
			}
		}
	}

	#[test]
	fn test_from_conversion() {
		// This is a compile-time test to ensure the From trait works
		fn _test_compile<E: Clone + Send + Sync + 'static>(sub: Subscriber) {
			let _receiver: NatsReceiver<E> = sub.into();
		}
	}
}

#![cfg(feature = "inmem")]

use crate::error::Result;

/// Core transport interface that all implementations must satisfy.
///
/// Provides a unified abstraction over event-based transports (e.g., WebSocket, NATS, etc.)
#[async_trait::async_trait]
pub trait Transport<E>: Clone + Send + Sync + 'static
where
	E: Clone + Send + Sync + 'static,
{
	/// Associated type for the receiver this transport produces
	type Receiver: Send + 'static;

	/// Opens a new dedicated channel for a specific connection key.
	async fn open_channel(&self, connection_key: &str) -> Self::Receiver;

	/// Closes the channel associated with a specific connection.
	async fn close_channel(&self, connection_key: &str) -> Result<()>;

	/// Sends an event directly to a specific connection.
	async fn send(&self, connection_key: &str, event: E) -> Result<()>;

	/// Broadcasts an event to all active connections.
	/// Returns the number of receivers that successfully received it.
	async fn broadcast(&self, event: E) -> Result<usize>;

	/// Subscribes to the global transport event stream.
	fn subscribe(&self) -> Self::Receiver;

	/// Returns the total number of active receivers.
	fn total_receivers(&self) -> usize;

	/// Returns whether the transport has been closed.
	fn is_closed(&self) -> bool;

	/// Returns the number of currently active connection channels.
	fn active_channels(&self) -> usize;
}

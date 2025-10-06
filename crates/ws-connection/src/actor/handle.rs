use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};

use super::command::ConnectionCommand;
use super::error::{ConnectionError, Result};
use super::state::ConnectionState;
use super::ConnectionActor;
use crate::core::conn::Connection;
use crate::core::subscription::EventKey;

/// Handle for communicating with a connection actor
#[derive(Clone, Debug)]
pub struct ConnectionHandle<K: EventKey> {
	pub connection: Arc<Connection<K>>,
	sender: mpsc::Sender<ConnectionCommand<K>>,
}

impl<K: EventKey> ConnectionHandle<K> {
	/// Create a new connection handle and actor pair.
	#[must_use]
	pub fn new(connection: Connection<K>, buffer_size: usize) -> (Self, ConnectionActor<K>) {
		let (sender, receiver) = mpsc::channel(buffer_size);
		let arc_conn = Arc::new(connection);

		let handle = Self {
			connection: arc_conn.clone(),
			sender,
		};
		let actor = ConnectionActor::new(arc_conn, receiver);
		(handle, actor)
	}

	/// Record recent activity.
	///
	/// # Errors
	/// Returns an error if the actor task is no longer available.
	pub async fn record_activity(&self) -> Result<()> {
		self.sender.send(ConnectionCommand::RecordActivity).await.map_err(|_| ConnectionError::ActorUnavailable)
	}

	/// Subscribe to the provided event types.
	///
	/// # Errors
	/// Returns an error if the actor task is no longer available.
	pub async fn subscribe(&self, event_types: Vec<K>) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::Subscribe { event_types })
			.await
			.map_err(|_| ConnectionError::ActorUnavailable)
	}

	/// Unsubscribe from the provided event types.
	///
	/// # Errors
	/// Returns an error if the actor task is no longer available.
	pub async fn unsubscribe(&self, event_types: Vec<K>) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::Unsubscribe { event_types })
			.await
			.map_err(|_| ConnectionError::ActorUnavailable)
	}

	/// Ask the actor to check for staleness based on timeout.
	///
	/// # Errors
	/// Returns an error if the actor task is no longer available.
	pub async fn check_stale(&self, timeout: Duration) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::CheckStale { timeout })
			.await
			.map_err(|_| ConnectionError::ActorUnavailable)
	}

	/// Mark the connection as stale.
	///
	/// # Errors
	/// Returns an error if the actor task is no longer available.
	pub async fn mark_stale(&self, reason: String) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::MarkStale { reason })
			.await
			.map_err(|_| ConnectionError::ActorUnavailable)
	}

	/// Disconnect the connection for the provided reason.
	///
	/// # Errors
	/// Returns an error if the actor task is no longer available.
	pub async fn disconnect(&self, reason: String) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::Disconnect { reason })
			.await
			.map_err(|_| ConnectionError::ActorUnavailable)
	}

	/// Get the current connection state.
	///
	/// # Errors
	/// Returns an error if the actor task is no longer available or fails to send back the state.
	pub async fn get_state(&self) -> Result<ConnectionState> {
		let (tx, rx) = oneshot::channel();
		self
			.sender
			.send(ConnectionCommand::GetState { reply: tx })
			.await
			.map_err(|_| ConnectionError::ActorUnavailable)?;

		rx.await.map_err(|_| ConnectionError::StateRetrievalFailed)
	}

	/// Request actor shutdown.
	///
	/// # Errors
	/// Returns an error if the actor task is no longer available.
	pub async fn shutdown(&self) -> Result<()> {
		self.sender.send(ConnectionCommand::Shutdown).await.map_err(|_| ConnectionError::ActorUnavailable)
	}

	/// Check if this handle's connection is subscribed to a given event type.
	#[must_use]
	pub fn is_subscribed_to(&self, event_type: &K) -> bool {
		self.connection.is_subscribed_to(event_type)
	}
}

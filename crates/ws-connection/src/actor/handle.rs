use std::{collections::HashSet, time::Duration};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::command::ConnectionCommand;
use super::error::{ConnectionError, Result};
use super::state::ConnectionState;
use super::ConnectionActor;
use crate::core::conn::Connection;
use crate::core::subscription::EventKey;

/// Handle for communicating with a connection actor
#[derive(Clone, Debug)]
pub struct ConnectionHandle<K: EventKey> {
	// Cache immutable data for quick access
	pub connection: Connection,
	sender: mpsc::Sender<ConnectionCommand<K>>,
	cancel_token: CancellationToken,
}

impl<K: EventKey> ConnectionHandle<K> {
	/// Create a new connection handle and actor pair
	#[must_use]
	pub fn new(connection: Connection, buffer_size: usize, parent_token: &CancellationToken) -> (Self, ConnectionActor<K>, CancellationToken) {
		let (sender, receiver) = mpsc::channel(buffer_size);

		let token = parent_token.child_token();

		let handle = Self {
			connection: connection.clone(),
			sender,
			cancel_token: token.clone(),
		};

		let conn_id = connection.id;
		let actor = ConnectionActor::new(conn_id, receiver);
		(handle, actor, token)
	}

	/// Record recent activity
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed.
	pub async fn record_activity(&self) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::RecordActivity)
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Subscribe to event types
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed.
	pub async fn subscribe(&self, event_types: Vec<K>) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::Subscribe { event_types })
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Unsubscribe from event types
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed.
	pub async fn unsubscribe(&self, event_types: Vec<K>) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::Unsubscribe { event_types })
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Check if subscribed to an event type
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed, or if the actor drops the
	/// reply channel before answering.
	pub async fn is_subscribed_to(&self, event_type: K) -> Result<bool> {
		let (tx, rx) = oneshot::channel();
		self
			.sender
			.send(ConnectionCommand::IsSubscribedTo { event_type, reply: tx })
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))?;

		rx.await.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Get all subscriptions
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed, or if the actor drops the
	/// reply channel before answering.
	pub async fn get_subscriptions(&self) -> Result<HashSet<K>> {
		let (tx, rx) = oneshot::channel();
		self
			.sender
			.send(ConnectionCommand::GetSubscriptions { reply: tx })
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))?;

		rx.await.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Check if connection should be marked stale
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed.
	pub async fn check_stale(&self, timeout: Duration) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::CheckStale { timeout })
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Mark connection as stale
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed.
	pub async fn mark_stale(&self, reason: String) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::MarkStale { reason })
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Disconnect the connection
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed.
	pub async fn disconnect(&self, reason: String) -> Result<()> {
		self
			.sender
			.send(ConnectionCommand::Disconnect { reason })
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Get current connection state
	///
	/// # Errors
	///
	/// Returns [`ConnectionError::ActorUnavailable`] if the actor has already
	/// shut down and its command channel is closed, or if the actor drops the
	/// reply channel before answering.
	pub async fn get_state(&self) -> Result<ConnectionState> {
		let (tx, rx) = oneshot::channel();
		self
			.sender
			.send(ConnectionCommand::GetState { reply: tx })
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))?;

		rx.await.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)))
	}

	/// Request actor shutdown
	///
	/// # Errors
	///
	/// Currently infallible: a closed command channel means the actor is
	/// already gone, which satisfies the request, so the cancellation token is
	/// still tripped and `Ok(())` returned. The signature stays fallible so
	/// shutdown can start reporting failures without a breaking change.
	pub async fn shutdown(&self) -> Result<()> {
		let _ = self
			.sender
			.send(ConnectionCommand::Shutdown)
			.await
			.map_err(|e| ConnectionError::ActorUnavailable(Box::new(e)));
		self.cancel_token.cancel();
		Ok(())
	}
}

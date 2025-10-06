pub mod command;
pub mod error;
pub mod handle;
pub mod state;

pub use command::ConnectionCommand;
pub use error::{ConnectionError, Result};
pub use handle::ConnectionHandle;
pub use state::ConnectionState;

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing;

use crate::core::conn::Connection;
use crate::core::subscription::EventKey;

/// Connection actor that owns its mutable state
pub struct ConnectionActor<K: EventKey> {
	connection: Arc<Connection<K>>,
	state: ConnectionState,
	commands: mpsc::Receiver<ConnectionCommand<K>>,
}

impl<K: EventKey> ConnectionActor<K> {
	/// Create a new connection actor with the given connection and command receiver.
	#[must_use]
	pub fn new(connection: Arc<Connection<K>>, commands: mpsc::Receiver<ConnectionCommand<K>>) -> Self {
		Self {
			connection,
			state: ConnectionState::new(),
			commands,
		}
	}

	/// Run the actor event loop.
	///
	/// # Panics
	/// Panics if the actor unexpectedly holds multiple `Arc` references to the same connection,
	/// since the actor requires unique ownership to mutate the underlying `Connection`.
	pub async fn run(mut self) {
		while let Some(cmd) = self.commands.recv().await {
			match cmd {
				ConnectionCommand::RecordActivity => {
					self.state.record_activity();
				}

				ConnectionCommand::Subscribe { event_types } => {
					if let Some(conn) = Arc::get_mut(&mut self.connection) {
						let change = conn.subscriptions.subscribe(event_types);
						if change.added > 0 {
							tracing::debug!("Connection {} subscribed to {} events", conn.id, change.added);
						}
					} else {
						tracing::error!("Multiple Arc references to connection detected during subscribe");
					}
				}

				ConnectionCommand::Unsubscribe { event_types } => {
					if let Some(conn) = Arc::get_mut(&mut self.connection) {
						let change = conn.subscriptions.unsubscribe(event_types);
						if change.removed > 0 {
							tracing::debug!("Connection {} unsubscribed from {} events", conn.id, change.removed);
						}
					} else {
						tracing::error!("Multiple Arc references to connection detected during unsubscribe");
					}
				}

				ConnectionCommand::CheckStale { timeout } => {
					if self.state.should_be_stale(timeout) {
						self.state.mark_stale("timeout".to_string());
						tracing::info!("Connection {} marked as stale", self.connection.id);
					}
				}

				ConnectionCommand::MarkStale { reason } => {
					self.state.mark_stale(reason);
				}

				ConnectionCommand::Disconnect { reason } => {
					self.state.disconnect(reason);
					tracing::info!("Connection {} disconnected", self.connection.id);
					break; // Exit actor loop
				}

				ConnectionCommand::GetState { reply } => {
					let _ = reply.send(self.state.clone());
				}

				ConnectionCommand::Shutdown => {
					tracing::debug!("Connection {} actor shutting down", self.connection.id);
					break;
				}
			}
		}
	}
}

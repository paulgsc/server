use tokio::sync::mpsc;
use tracing;

use crate::core::subscription::{EventKey, SubscriptionManager};

pub mod command;
pub mod error;
pub mod handle;
pub mod state;

use crate::types::ConnectionId;
pub use command::ConnectionCommand;
pub use error::{ConnectionError, Result};
pub use handle::ConnectionHandle;
pub use state::ConnectionState;

/// Connection actor that owns mutable state (subscriptions + connection state)
pub struct ConnectionActor<K: EventKey> {
	id: ConnectionId,
	subscriptions: SubscriptionManager<K>, // Mutable, actor-managed
	state: ConnectionState,                // Mutable, actor-managed
	commands: mpsc::Receiver<ConnectionCommand<K>>,
}

impl<K: EventKey> ConnectionActor<K> {
	/// Create a new connection actor
	#[must_use]
	pub fn new(id: ConnectionId, commands: mpsc::Receiver<ConnectionCommand<K>>) -> Self {
		Self {
			id,
			subscriptions: SubscriptionManager::new(),
			state: ConnectionState::new(),
			commands,
		}
	}

	/// Run the actor event loop
	pub async fn run(mut self) {
		while let Some(cmd) = self.commands.recv().await {
			match cmd {
				ConnectionCommand::RecordActivity => {
					self.state.record_activity();
				}

				ConnectionCommand::Subscribe { event_types } => {
					let change = self.subscriptions.subscribe(event_types);
					if change.added > 0 {
						tracing::debug!("Connection {} subscribed to {} events", self.id, change.added);
					}
				}

				ConnectionCommand::Unsubscribe { event_types } => {
					let change = self.subscriptions.unsubscribe(event_types);
					if change.removed > 0 {
						tracing::debug!("Connection {} unsubscribed from {} events", self.id, change.removed);
					}
				}

				ConnectionCommand::IsSubscribedTo { event_type, reply } => {
					let result = self.subscriptions.is_subscribed_to(&event_type);
					let _ = reply.send(result);
				}

				ConnectionCommand::GetSubscriptions { reply } => {
					let subs = self.subscriptions.get_subscriptions().clone();
					let _ = reply.send(subs);
				}

				ConnectionCommand::CheckStale { timeout } => {
					if self.state.should_be_stale(timeout) {
						self.state.mark_stale("timeout".to_string());
						tracing::info!("Connection {} marked as stale", self.id);
					}
				}

				ConnectionCommand::MarkStale { reason } => {
					self.state.mark_stale(reason);
				}

				ConnectionCommand::Disconnect { reason } => {
					self.state.disconnect(reason);
					tracing::info!("Connection {} disconnected", self.id);
					break;
				}

				ConnectionCommand::GetState { reply } => {
					let _ = reply.send(self.state.clone());
				}

				ConnectionCommand::Shutdown => {
					tracing::debug!("Connection {} actor shutting down", self.id);
					break;
				}
			}
		}
	}
}

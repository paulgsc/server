use std::time::Duration;
use tokio::sync::oneshot;

use super::state::ConnectionState;
use crate::core::subscription::EventKey;

/// Messages that can be sent to a connection actor
#[derive(Debug)]
pub enum ConnectionCommand<K: EventKey> {
	/// Record activity (ping received)
	RecordActivity,

	/// Subscribe to events
	Subscribe { event_types: Vec<K> },

	/// Unsubscribe from events
	Unsubscribe { event_types: Vec<K> },

	/// Check if should be marked stale
	CheckStale { timeout: Duration },

	/// Mark as stale
	MarkStale { reason: String },

	/// Disconnect
	Disconnect { reason: String },

	/// Get current state
	GetState { reply: oneshot::Sender<ConnectionState> },

	/// Shutdown the actor
	Shutdown,
}

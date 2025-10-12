use std::{collections::HashSet, time::Duration};
use tokio::sync::oneshot;

use super::state::ConnectionState;
use crate::core::subscription::EventKey;

/// Messages that can be sent to a connection actor
#[derive(Debug)]
pub enum ConnectionCommand<K: EventKey> {
	RecordActivity,

	Subscribe { event_types: Vec<K> },

	Unsubscribe { event_types: Vec<K> },

	IsSubscribedTo { event_type: K, reply: oneshot::Sender<bool> },

	GetSubscriptions { reply: oneshot::Sender<HashSet<K>> },

	CheckStale { timeout: Duration },

	MarkStale { reason: String },

	Disconnect { reason: String },

	GetState { reply: oneshot::Sender<ConnectionState> },

	Shutdown,
}

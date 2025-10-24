#![cfg(feature = "events")]

// System event for observability and debugging
#[derive(Debug, Clone)]
pub enum SystemEvent {
	ConnectionStateChanged {
		connection_id: ConnectionId,
		from: ConnectionState,
		to: ConnectionState,
	},
	MessageProcessed {
		message_id: MessageId,
		connection_id: ConnectionId,
		duration: Duration,
		result: ProcessResult,
	},
	BroadcastFailed {
		event_type: EventType,
		error: String,
		affected_connections: usize,
	},
	ConnectionCleanup {
		connection_id: ConnectionId,
		reason: String,
		resources_freed: bool,
	},
}

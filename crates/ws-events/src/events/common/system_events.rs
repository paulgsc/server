use super::{EventType, MessageId, ProcessResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "eventType")]
#[serde(rename_all = "camelCase")]
pub enum SystemEvent {
	ConnectionStateChanged {
		connection_id: String,
		from: String,
		to: String,
		#[serde(default)]
		metadata: serde_json::Value,
	},
	MessageProcessed {
		message_id: MessageId,
		connection_id: String,
		result: ProcessResult,
		#[serde(default)]
		metadata: serde_json::Value,
	},
	BroadcastFailed {
		event_type: EventType,
		error: String,
		affected_connections: u64,
		#[serde(default)]
		metadata: serde_json::Value,
	},
	ConnectionCleanup {
		connection_id: String,
		reason: String,
		resources_freed: bool,
		#[serde(default)]
		metadata: serde_json::Value,
	},
	// Catch-all for future events
	#[serde(untagged)]
	Other {
		name: String,
		#[serde(flatten)]
		data: serde_json::Value,
	},
}

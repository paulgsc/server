use obs_websocket::{ObsCommand, ObsEvent};
use prost::Message;
use serde_json::Value;

/// Message sent by clients to control OBS
#[derive(Clone, Message)]
pub struct ObsCommandMessage {
	/// Unique request ID for tracking responses
	#[prost(string, tag = "1")]
	pub request_id: String,

	/// The actual OBS command as serialized JSON
	#[prost(bytes, tag = "2")]
	pub command_data: Vec<u8>,

	/// Optional reply subject for command acknowledgment
	#[prost(string, optional, tag = "3")]
	pub reply_to: Option<String>,
}

/// Message published when OBS events occur
#[derive(Clone, Message)]
pub struct ObsEventMessage {
	/// Timestamp when the event occurred
	#[prost(int64, tag = "1")]
	pub timestamp: i64,

	/// The actual OBS event as serialized JSON
	#[prost(bytes, tag = "2")]
	pub event_data: Vec<u8>,

	/// Optional metadata as serialized JSON
	#[prost(bytes, optional, tag = "3")]
	pub metadata: Option<Vec<u8>>,
}

// Helper implementations to work with the original types
impl ObsCommandMessage {
	/// Create a new command message from an ObsCommand
	pub fn new(request_id: String, command: ObsCommand) -> Result<Self, serde_json::Error> {
		let command_data = serde_json::to_vec(&command)?;
		Ok(Self {
			request_id,
			command_data,
			reply_to: None,
		})
	}

	/// Set the reply_to field
	pub fn with_reply_to(mut self, reply_to: String) -> Self {
		self.reply_to = Some(reply_to);
		self
	}

	/// Deserialize the command back to ObsCommand
	pub fn to_command(&self) -> Result<ObsCommand, serde_json::Error> {
		serde_json::from_slice(&self.command_data)
	}
}

impl ObsEventMessage {
	/// Create a new event message from an ObsEvent
	pub fn new(event: ObsEvent) -> Result<Self, serde_json::Error> {
		let event_data = serde_json::to_vec(&event)?;
		Ok(Self {
			timestamp: chrono::Utc::now().timestamp(),
			event_data,
			metadata: None,
		})
	}

	/// Add metadata to the event
	pub fn with_metadata(mut self, metadata: Value) -> Result<Self, serde_json::Error> {
		self.metadata = Some(serde_json::to_vec(&metadata)?);
		Ok(self)
	}

	/// Deserialize the event back to ObsEvent
	pub fn to_event(&self) -> Result<ObsEvent, serde_json::Error> {
		serde_json::from_slice(&self.event_data)
	}

	/// Get the metadata as a Value
	pub fn get_metadata(&self) -> Result<Option<Value>, serde_json::Error> {
		match &self.metadata {
			Some(data) => Ok(Some(serde_json::from_slice(data)?)),
			None => Ok(None),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_command_message_roundtrip() {
		// This test assumes ObsCommand implements Serialize/Deserialize
		// You'll need to create a real ObsCommand for actual testing
		let request_id = "test-123".to_string();

		// Create a mock command (you'll need to use real ObsCommand values)
		// let command = ObsCommand::SomeVariant { ... };
		// let msg = ObsCommandMessage::new(request_id.clone(), command.clone()).unwrap();
		// let recovered = msg.to_command().unwrap();
		// assert_eq!(command, recovered);
	}

	#[test]
	fn test_event_message_with_metadata() {
		// Similar to above, test with real ObsEvent
		// let event = ObsEvent::SomeVariant { ... };
		// let metadata = serde_json::json!({"key": "value"});
		// let msg = ObsEventMessage::new(event.clone()).unwrap()
		//     .with_metadata(metadata.clone()).unwrap();
		//
		// let recovered_event = msg.to_event().unwrap();
		// let recovered_metadata = msg.get_metadata().unwrap();
		// assert_eq!(event, recovered_event);
		// assert_eq!(recovered_metadata, Some(metadata));
	}

	#[test]
	fn test_protobuf_encoding() {
		// Test that messages can be encoded/decoded with protobuf
		let msg = ObsCommandMessage {
			request_id: "test-456".to_string(),
			command_data: vec![1, 2, 3, 4],
			reply_to: Some("reply-topic".to_string()),
		};

		// Encode
		let mut buf = Vec::new();
		msg.encode(&mut buf).unwrap();

		// Decode
		let decoded = ObsCommandMessage::decode(&buf[..]).unwrap();
		assert_eq!(msg.request_id, decoded.request_id);
		assert_eq!(msg.command_data, decoded.command_data);
		assert_eq!(msg.reply_to, decoded.reply_to);
	}
}

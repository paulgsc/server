#![cfg(feature = "events")]

use obs_websocket::{ObsCommand, ObsEvent};
use prost::Message;
use serde_json::Value;

#[derive(Clone, PartialEq, Message)]
pub struct ObsStatusMessage {
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

#[derive(Clone, PartialEq, Message)]
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

impl ObsStatusMessage {
	/// Create a new status message from an ObsEvent
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
	pub fn to_obs_event(&self) -> Result<ObsEvent, serde_json::Error> {
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
	pub fn to_obs_command(&self) -> Result<ObsCommand, serde_json::Error> {
		serde_json::from_slice(&self.command_data)
	}
}

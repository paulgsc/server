use crate::messages::{EventMessageParser, HelloMessageParser, JsonExtractor, ObsEvent, ObsMessagesError, ResponseMessageParser};
use serde_json::Value;
use std::collections::HashMap;
use tracing::{error, instrument, trace};

type Result<T> = std::result::Result<T, ObsMessagesError>;

pub struct ObsMessageProcessor {
	/// Cache for tracking message statistics
	message_stats: HashMap<String, u64>,
}

impl ObsMessageProcessor {
	pub fn new() -> Self {
		Self { message_stats: HashMap::new() }
	}

	/// Process an OBS WebSocket message and return the appropriate event
	#[instrument(skip(self, text), fields(message_len = text.len()))]
	pub async fn process_message(&mut self, text: String) -> Result<ObsEvent> {
		// Parse JSON with detailed error handling
		let json: Value = serde_json::from_str(&text).map_err(|e| {
			error!("Failed to parse JSON from OBS message: {}", e);
			trace!("Failed message content (first 200 chars): {}", text.chars().take(200).collect::<String>());
			ObsMessagesError::JsonParse(e)
		})?;

		// Extract operation code
		let op = self.extract_operation_code(&json, &text)?;

		// Update statistics
		self.update_message_stats(op);

		// Process based on operation code
		let event = match op {
			0 => {
				trace!("Processing Hello message (op: 0)");
				HelloMessageParser::parse(&json)?
			}
			2 => {
				trace!("Processing Identified message (op: 2)");
				ObsEvent::Identified
			}
			5 => {
				trace!("Processing Event message (op: 5)");
				EventMessageParser::parse(&json)?
			}
			7 => {
				trace!("Processing Response message (op: 7)");
				ResponseMessageParser::parse(&json)?
			}
			_ => {
				error!("Unknown operation code {} in message", op);
				return Err(ObsMessagesError::UnknownOpCode { op_code: op });
			}
		};

		trace!("Successfully processed OBS message with op code: {}", op);
		Ok(event)
	}

	/// Extract and validate the operation code from the JSON message
	fn extract_operation_code(&self, json: &Value, original_text: &str) -> Result<u64> {
		let _extractor = JsonExtractor::new(json, "OBS message");

		match json.get("op") {
			Some(op_value) => op_value.as_u64().ok_or_else(|| {
				error!("Invalid 'op' field type in message: expected number, got {:?}", op_value);
				trace!("Message content: {}", original_text);
				ObsMessagesError::InvalidFieldType {
					field: "op".to_string(),
					expected: "number".to_string(),
				}
			}),
			None => {
				error!("Missing 'op' field in OBS message");
				trace!("Message content: {}", original_text);
				Err(ObsMessagesError::MissingField {
					field: "op".to_string(),
					message_type: "OBS message".to_string(),
				})
			}
		}
	}

	/// Update internal message processing statistics
	fn update_message_stats(&mut self, op_code: u64) {
		let op_key = format!("op_{}", op_code);
		*self.message_stats.entry(op_key).or_insert(0) += 1;

		trace!("Message stats updated. Current counts: {:?}", self.message_stats);
	}

	/// Get current message processing statistics
	pub fn get_message_stats(&self) -> &HashMap<String, u64> {
		&self.message_stats
	}

	/// Reset message processing statistics
	pub fn reset_stats(&mut self) {
		trace!("Resetting message processing statistics");
		self.message_stats.clear();
	}
}

impl Default for ObsMessageProcessor {
	fn default() -> Self {
		Self::new()
	}
}

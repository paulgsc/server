use crate::messages::ObsMessagesError;
use serde_json::Value;
use tracing::trace;

type Result<T> = std::result::Result<T, ObsMessagesError>;

/// Extracts and validates JSON fields with proper error handling
pub(crate) struct JsonExtractor<'a> {
	json: &'a Value,
	context: String,
}

impl<'a> JsonExtractor<'a> {
	pub fn new(json: &'a Value, context: impl Into<String>) -> Self {
		Self { json, context: context.into() }
	}

	pub fn get_object(&self, field: &str) -> Result<&serde_json::Map<String, Value>> {
		self
			.json
			.get(field)
			.ok_or_else(|| {
				trace!("Missing field '{}' in {}", field, self.context);
				ObsMessagesError::MissingField {
					field: field.to_string(),
					message_type: self.context.clone(),
				}
			})?
			.as_object()
			.ok_or_else(|| {
				trace!("Field '{}' is not an object in {}", field, self.context);
				ObsMessagesError::InvalidFieldType {
					field: field.to_string(),
					expected: "object".to_string(),
				}
			})
	}

	pub fn get_string(&self, field: &str) -> Result<&str> {
		self
			.json
			.get(field)
			.ok_or_else(|| {
				trace!("Missing field '{}' in {}", field, self.context);
				ObsMessagesError::MissingField {
					field: field.to_string(),
					message_type: self.context.clone(),
				}
			})?
			.as_str()
			.ok_or_else(|| {
				trace!("Field '{}' is not a string in {}", field, self.context);
				ObsMessagesError::InvalidFieldType {
					field: field.to_string(),
					expected: "string".to_string(),
				}
			})
	}

	#[allow(dead_code)]
	fn get_u64(&self, field: &str) -> Result<u64> {
		self
			.json
			.get(field)
			.ok_or_else(|| {
				trace!("Missing field '{}' in {}", field, self.context);
				ObsMessagesError::MissingField {
					field: field.to_string(),
					message_type: self.context.clone(),
				}
			})?
			.as_u64()
			.ok_or_else(|| {
				trace!("Field '{}' is not a number in {}", field, self.context);
				ObsMessagesError::InvalidFieldType {
					field: field.to_string(),
					expected: "number".to_string(),
				}
			})
	}
}

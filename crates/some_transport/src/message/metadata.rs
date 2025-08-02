use crate::protocol::types::{ContentEncoding, DeliveryMode};
use tokio::time::Instant;

#[derive(Debug, Clone)]
pub struct MessageId(String);

impl MessageId {
	pub fn new() -> Self {
		Self(uuid::Uuid::new_v4().to_string())
	}
}

#[derive(Debug, Clone)]
pub struct CorrelationId(String);

#[derive(Debug, Clone)]
pub struct MessageMetadata {
	pub message_id: MessageId,
	pub timestamp: Instant,
	pub priority: MessagePriority,
	pub correlation_id: Option<CorrelationId>,
	pub content_encoding: Option<ContentEncoding>,
	pub delivery_mode: DeliveryMode,
}

impl Default for MessageMetadata {
	fn default() -> Self {
		Self {
			message_id: MessageId::new(),
			timestamp: Instant::now(), // Fresh timestamp each time
			priority: MessagePriority::Normal,
			correlation_id: None,
			content_encoding: None,
			delivery_mode: DeliveryMode::BestEffort,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
	Low = 0,
	Normal = 1,
	High = 2,
	Critical = 3,
}

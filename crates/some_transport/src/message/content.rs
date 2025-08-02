use crate::message::metadata::{CorrelationId, MessageMetadata, MessagePriority};
use crate::protocol::types::CloseCode;
use crate::transport::error::TransportError;
use bytes::Bytes;
use tokio::sync::oneshot;
use tokio::time::Duration;

#[derive(Debug)]
pub enum MessageContent {
	Text(String),
	Binary(Bytes),
}

#[derive(Debug)]
pub struct OutgoingMessage {
	pub content: MessageContent,
	pub priority: MessagePriority,
	pub timeout: Option<Duration>,
	pub correlation_id: Option<CorrelationId>,
	pub response_channel: Option<oneshot::Sender<Result<TransportMessage, TransportError>>>,
}

#[derive(Debug, Clone)]
pub enum TransportMessage {
	Text { data: String, metadata: MessageMetadata },
	Binary { data: Bytes, metadata: MessageMetadata },
	Ping { data: Option<Bytes> },
	Pong { data: Option<Bytes> },
	Close { code: Option<CloseCode>, reason: Option<String> },
}

impl TransportMessage {
	pub fn message_type(&self) -> &'static str {
		match self {
			Self::Text { .. } => "text",
			Self::Binary { .. } => "binary",
			Self::Ping { .. } => "ping",
			Self::Pong { .. } => "pong",
			Self::Close { .. } => "close",
		}
	}

	pub fn metadata(&self) -> &MessageMetadata {
		match self {
			Self::Text { metadata, .. } | Self::Binary { metadata, .. } => metadata,
			_ => {
				// For control messages, return a reference to a static default
				static DEFAULT_METADATA: std::sync::LazyLock<MessageMetadata> = std::sync::LazyLock::new(|| MessageMetadata::default());
				&DEFAULT_METADATA
			}
		}
	}

	pub fn should_broadcast(&self) -> bool {
		matches!(self, Self::Text { .. } | Self::Binary { .. })
	}
}

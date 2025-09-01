mod config;
mod error;
mod extractor;
mod parsers;
mod processor;
mod types;

use futures_util::{sink::SinkExt, stream::SplitSink};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream};
use tracing::{error, instrument, trace};

// Assume these are imported from your types module
use config::InitializationConfig;
use error::ObsMessagesError;
use extractor::JsonExtractor;
use parsers::{EventMessageParser, HelloMessageParser, ResponseMessageParser};
use processor::ObsMessageProcessor;
use types::HelloData;
pub use types::{ObsEvent, ObsRequestType};

type Result<T> = std::result::Result<T, ObsMessagesError>;
type WebSocketSink = SplitSink<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>;

/// Handles initialization of OBS WebSocket connection (stateless)
pub struct ObsInitializer {
	config: InitializationConfig,
}

impl ObsInitializer {
	pub fn new(config: InitializationConfig) -> Self {
		Self { config }
	}

	/// Send initial state requests to OBS
	#[instrument(skip(self, sink), fields(request_count = self.config.requests.len()))]
	pub async fn fetch_init_state(&self, sink: &mut WebSocketSink) -> Result<()> {
		trace!("Starting OBS initialization with {} requests", self.config.requests.len());

		for (request_type, request_id) in &self.config.requests {
			let request = json!({
				"op": 6,
				"d": {
					"requestType": request_type.as_str(),
					"requestId": request_id
				}
			});

			trace!("Sending initialization request: {} ({})", request_type.as_str(), request_id);

			sink.send(TungsteniteMessage::Text(request.to_string().into())).await.map_err(|e| {
				error!("Failed to send initialization request {}: {}", request_type.as_str(), e);
				ObsMessagesError::WebSocketSend(e)
			})?;
		}

		sink.flush().await.map_err(|e| {
			error!("Failed to flush WebSocket sink during initialization: {}", e);
			ObsMessagesError::WebSocketSend(e)
		})?;

		trace!("OBS initialization requests sent successfully");
		Ok(())
	}
}

/// Thread-safe message processor wrapper
pub struct MessageProcessor {
	processor: Arc<Mutex<ObsMessageProcessor>>,
}

impl MessageProcessor {
	pub fn new() -> Self {
		Self {
			processor: Arc::new(Mutex::new(ObsMessageProcessor::new())),
		}
	}

	/// Process an incoming OBS message (thread-safe)
	#[instrument(skip(self, message))]
	pub async fn process_message(&self, message: String) -> Result<ObsEvent> {
		let mut processor = self.processor.lock().await;
		let event = processor.process_message(message).await?;

		Ok(event)
	}

	/// Get processing statistics (thread-safe)
	pub async fn get_stats(&self) -> HashMap<String, u64> {
		let processor = self.processor.lock().await;
		processor.get_message_stats().clone()
	}

	/// Reset processing statistics (thread-safe)
	pub async fn reset_stats(&self) {
		let mut processor = self.processor.lock().await;
		processor.reset_stats();
	}

	/// Clone for sharing across tasks
	pub fn clone(&self) -> Self {
		Self {
			processor: Arc::clone(&self.processor),
		}
	}
}

impl Default for MessageProcessor {
	fn default() -> Self {
		Self::new()
	}
}

/// High-level facade combining initialization and processing
pub struct MessageHandler {
	initializer: ObsInitializer,
	processor: MessageProcessor,
}

impl MessageHandler {
	pub fn new() -> Self {
		Self {
			initializer: ObsInitializer::new(InitializationConfig::default()),
			processor: MessageProcessor::new(),
		}
	}

	pub fn with_config(config: InitializationConfig) -> Self {
		Self {
			initializer: ObsInitializer::new(config),
			processor: MessageProcessor::new(),
		}
	}

	/// Initialize the OBS WebSocket connection
	#[instrument(skip(self, sink))]
	pub async fn initialize(&self, sink: &mut WebSocketSink) -> Result<()> {
		trace!("Starting OBS WebSocket initialization");
		self.initializer.fetch_init_state(sink).await?;
		trace!("OBS WebSocket initialization completed successfully");
		Ok(())
	}

	/// Get a cloneable processor for use in async tasks
	pub fn processor(&self) -> MessageProcessor {
		self.processor.clone()
	}

	/// Process an incoming OBS message (convenience method)
	#[instrument(skip(self, message))]
	pub async fn process_message(&self, message: String) -> Result<ObsEvent> {
		let event = self.processor.process_message(message).await?;
		Ok(event)
	}

	/// Get processing statistics
	pub async fn get_stats(&self) -> HashMap<String, u64> {
		self.processor.get_stats().await
	}

	/// Reset processing statistics
	pub async fn reset_stats(&self) {
		self.processor.reset_stats().await
	}
}

impl Default for MessageHandler {
	fn default() -> Self {
		Self::new()
	}
}

use super::*;
use axum::extract::ws::{CloseFrame, WebSocket};
use futures::stream::SplitStream;
use tokio::time::{Duration, Instant};

// Enhanced message FSM with correlation tracking
#[derive(Debug)]
pub enum MessageState {
	Received { raw: String },
	Parsed { event: Event },
	Validated { event: Event },
	Processed { event: Event, result: ProcessResult },
	Failed { error: String },
}

#[derive(Debug, Clone)]
pub struct ProcessResult {
	pub delivered: usize,
	pub failed: usize,
	pub duration: Duration,
}

#[derive(Debug)]
pub struct EventMessage {
	pub id: MessageId,
	pub connection_id: ConnectionId,
	pub timestamp: Instant,
	pub state: MessageState,
}

impl EventMessage {
	pub fn new(connection_id: ConnectionId, raw: String) -> Self {
		Self {
			id: MessageId::new(),
			connection_id,
			timestamp: Instant::now(),
			state: MessageState::Received { raw },
		}
	}

	pub fn parse(&mut self) -> Result<(), String> {
		match &self.state {
			MessageState::Received { raw } => match serde_json::from_str::<Event>(raw) {
				Ok(event) => {
					self.state = MessageState::Parsed { event };
					Ok(())
				}
				Err(e) => {
					let error = format!("Parse error: {}", e);
					self.state = MessageState::Failed { error: error.clone() };
					Err(error)
				}
			},
			_ => Err("Can only parse received messages".to_string()),
		}
	}

	pub fn validate(&mut self) -> Result<(), String> {
		match &self.state {
			MessageState::Parsed { event } => match event {
				Event::Error { message } if message.is_empty() => {
					let error = "Error event cannot have empty message".to_string();
					self.state = MessageState::Failed { error: error.clone() };
					Err(error)
				}
				_ => {
					self.state = MessageState::Validated { event: event.clone() };
					Ok(())
				}
			},
			_ => Err("Can only validate parsed messages".to_string()),
		}
	}

	pub fn mark_processed(&mut self, result: ProcessResult) {
		if let MessageState::Validated { event } = &self.state {
			self.state = MessageState::Processed { event: event.clone(), result };
		}
	}

	pub fn get_event(&self) -> Option<&Event> {
		match &self.state {
			MessageState::Parsed { event } | MessageState::Validated { event } | MessageState::Processed { event, .. } => Some(event),
			_ => None,
		}
	}

	pub fn duration_since_creation(&self) -> Duration {
		Instant::now().duration_since(self.timestamp)
	}
}

// Processes all incoming messages from the WebSocket
pub(crate) async fn process_incoming_messages(mut receiver: SplitStream<WebSocket>, state: &WebSocketFsm, client_key: &str) -> u64 {
	let mut message_count = 0u64;

	while let Some(result) = receiver.next().await {
		message_count += 1;

		match result {
			Ok(msg) => {
				if let Err(_) = handle_websocket_message(msg, state, client_key, message_count).await {
					break;
				}
			}
			Err(e) => {
				record_ws_error!("websocket_error", "connection", e);
				error!("WebSocket error for {} (msg #{}): {}", client_key, message_count, e);
				break;
			}
		}
	}

	message_count
}

// Handles a single WebSocket message based on its type
async fn handle_websocket_message(msg: Message, state: &WebSocketFsm, client_key: &str, message_count: u64) -> Result<(), ()> {
	match msg {
		Message::Text(text) => handle_text_message(text, state, client_key, message_count).await,
		Message::Ping(_) => handle_ping_message(state, client_key).await,
		Message::Pong(_) => handle_pong_message(state, client_key).await,
		Message::Close(reason) => handle_close_message(client_key, reason).await,
		_ => {
			debug!("Ignored message type from {}", client_key);
			Ok(())
		}
	}
}

// Handles text messages from clients
async fn handle_text_message(text: String, state: &WebSocketFsm, client_key: &str, message_count: u64) -> Result<(), ()> {
	record_system_event!("message_received", connection_id = client_key, message_number = message_count, size_bytes = text.len());
	debug!("Received message #{} from {}: {} chars", message_count, client_key, text.len());

	let processing_result: Result<(), String> = timed_ws_operation!("websocket", "process_message", {
		state.process_message(client_key, text).await;
		Ok(())
	});

	if processing_result.is_err() {
		record_ws_error!("message_processing_failed", "websocket");
	}

	Ok(())
}

// Handles ping messages
async fn handle_ping_message(state: &WebSocketFsm, client_key: &str) -> Result<(), ()> {
	record_system_event!("ping_received", connection_id = client_key);
	debug!("Received WebSocket ping from {}", client_key);

	if let Err(e) = state.update_client_ping(client_key).await {
		record_ws_error!("ping_handling_failed", "websocket", e);
		warn!("Failed to update ping for {}: {}", client_key, e);
	}

	Ok(())
}

// Handles pong messages
async fn handle_pong_message(state: &WebSocketFsm, client_key: &str) -> Result<(), ()> {
	record_system_event!("pong_received", connection_id = client_key);
	debug!("Received WebSocket pong from {}", client_key);

	if let Err(e) = state.update_client_ping(client_key).await {
		record_ws_error!("pong_handling_failed", "websocket", e);
		warn!("Failed to update pong for {}: {}", client_key, e);
	}

	Ok(())
}

// Handles close messages
async fn handle_close_message(client_key: &str, reason: Option<CloseFrame<'_>>) -> Result<(), ()> {
	record_system_event!("close_received", connection_id = client_key, reason = reason);
	info!("Client {} closed connection: {:?}", client_key, reason);
	Err(()) // Signal to break the message processing loop
}

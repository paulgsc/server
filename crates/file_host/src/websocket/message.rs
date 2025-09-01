use super::*;
use crate::utils::retry::retry_async;
use axum::extract::ws::{CloseFrame, WebSocket};
use futures::stream::SplitStream;
use obs_websocket::ObsCommand;
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

impl WebSocketFsm {
	// Enhanced message processing with full traceability
	pub async fn process_message(&self, conn_key: &str, raw_message: String) {
		// Get connection ID for correlation
		let connection_id = if let Some(conn) = self.connections.get(conn_key) {
			conn.id.clone()
		} else {
			record_ws_error!("connection_not_found", "message_processing");
			error!("Cannot process message for unknown client: {}", conn_key);
			return;
		};

		let mut message = EventMessage::new(connection_id.clone(), raw_message);

		// Update message count
		if let Some(mut conn) = self.connections.get_mut(conn_key) {
			conn.increment_message_count();
		}

		// Parse
		if let Err(e) = message.parse() {
			record_message_result!("unknown", "parse_failed", connection_id: connection_id);
			record_ws_error!("parse_error", "message", e);
			self.metrics.message_processed(false);
			self.send_error_to_client(conn_key, &e).await;
			return;
		}

		// Handle control messages immediately
		if let Some(event) = message.get_event() {
			let event_type_str = format!("{:?}", event.get_type());

			match event {
				Event::Pong => {
					record_message_result!(&event_type_str, "success", connection_id: connection_id);
					if let Err(e) = self.update_client_ping(conn_key).await {
						record_ws_error!("ping_update_failed", "connection", e);
					}
					self.metrics.message_processed(true);
					return;
				}
				Event::Subscribe { event_types } => {
					record_message_result!(&event_type_str, "success", connection_id: connection_id);
					self.handle_subscription(conn_key, event_types.clone(), true).await;
					self.metrics.message_processed(true);
					return;
				}
				Event::Unsubscribe { event_types } => {
					record_message_result!(&event_type_str, "success", connection_id: connection_id);
					self.handle_subscription(conn_key, event_types.clone(), false).await;
					self.metrics.message_processed(true);
					return;
				}
				_ => {}
			}
		}

		// Validate
		if let Err(e) = message.validate() {
			record_message_result!("unknown", "validation_failed", connection_id: connection_id);
			record_ws_error!("validation_error", "message", e);
			self.metrics.message_processed(false);
			self.send_error_to_client(conn_key, &e).await;
			return;
		}

		// Process (broadcast)
		if let Some(event) = message.get_event() {
			let event_type_str = format!("{:?}", event.get_type());
			let start_time = Instant::now();

			let result = match event {
				Event::ObsCmd { cmd } => {
					// Handle OBS commands with graceful failure and retry
					self.handle_obs_command(cmd.clone(), &connection_id).await;

					// Return success immediately - command is handled asynchronously
					ProcessResult {
						delivered: 1,
						failed: 0,
						duration: start_time.elapsed(),
					}
				}
				_ => {
					// Handle other events (broadcasting)
					timed_ws_operation!(&event_type_str, "process", { self.broadcast_event(event).await })
				}
			};

			let duration = start_time.elapsed();
			let process_result = ProcessResult {
				delivered: result.delivered,
				failed: result.failed,
				duration,
			};

			message.mark_processed(process_result.clone());
			record_message_result!(&event_type_str, "success", connection_id: connection_id);
			self.metrics.message_processed(true);

			// Emit system event for monitoring
			record_system_event!(
				"message_processed",
				message_id = message.id,
				connection_id = connection_id,
				delivered = process_result.delivered,
				failed = process_result.failed,
				duration_ms = duration.as_millis()
			);
		}
	}

	// Non-blocking OBS command handler with retry logic
	async fn handle_obs_command(&self, cmd: ObsCommand, connection_id: &ConnectionId) {
		// Clone necessary data for the async task
		let obs_manager = self.obs_manager.clone(); // Assuming you have access to ObsWebSocketManager
		let connection_id = connection_id.to_string();

		// Spawn non-blocking task for command execution
		tokio::spawn(async move {
			let cmd_clone = cmd.clone();
			let start_time = Instant::now();
			let result = retry_async(
				|| obs_manager.execute_command(cmd_clone.clone()),
				3,                          // max attempts
				Duration::from_millis(100), // base backoff
				2,                          // exponential factor
			)
			.await;

			match result {
				Ok(_) => {
					// log success
					let duration = start_time.elapsed();
					record_system_event!(
						"obs_command_success",
						connection_id = connection_id,
						command = format!("{:?}", cmd),
						duration_ms = duration.as_millis()
					);
				}
				Err(e) => {
					// log final failure
					let duration = start_time.elapsed();
					record_ws_error!("obs_command_final_failure", "command_execution", &e);
					record_system_event!(
						"obs_command_failed",
						connection_id = connection_id,
						command = format!("{:?}", cmd),
						error = e.to_string(),
						duration_ms = duration.as_millis()
					);
				}
			}
		});
	}
}

// Processes all incoming messages from the WebSocket
pub(crate) async fn process_incoming_messages(mut receiver: SplitStream<WebSocket>, state: &WebSocketFsm, conn_key: &str) -> u64 {
	let mut message_count = 0u64;

	while let Some(result) = receiver.next().await {
		message_count += 1;

		match result {
			Ok(msg) => {
				if let Err(_) = handle_websocket_message(msg, state, conn_key, message_count).await {
					break;
				}
			}
			Err(e) => {
				record_ws_error!("websocket_error", "connection", e);
				error!("WebSocket error for {} (msg #{}): {}", conn_key, message_count, e);
				break;
			}
		}
	}

	message_count
}

// Handles a single WebSocket message based on its type
async fn handle_websocket_message(msg: Message, state: &WebSocketFsm, conn_key: &str, message_count: u64) -> Result<(), ()> {
	match msg {
		Message::Text(text) => handle_text_message(text, state, conn_key, message_count).await,
		Message::Ping(_) => handle_ping_message(state, conn_key).await,
		Message::Pong(_) => handle_pong_message(state, conn_key).await,
		Message::Close(reason) => handle_close_message(state, conn_key, reason).await,
		_ => {
			debug!("Ignored message type from {}", conn_key);
			Ok(())
		}
	}
}

// Handles text messages from clients
async fn handle_text_message(text: String, state: &WebSocketFsm, conn_key: &str, message_count: u64) -> Result<(), ()> {
	record_system_event!("message_received", connection_id = conn_key, message_number = message_count, size_bytes = text.len());
	debug!("Received message #{} from {}: {} chars", message_count, conn_key, text.len());

	let processing_result: Result<(), String> = timed_ws_operation!("websocket", "process_message", {
		state.process_message(conn_key, text).await;
		Ok(())
	});

	if processing_result.is_err() {
		record_ws_error!("message_processing_failed", "websocket");
	}

	Ok(())
}

// Handles ping messages
async fn handle_ping_message(state: &WebSocketFsm, conn_key: &str) -> Result<(), ()> {
	record_system_event!("ping_received", connection_id = conn_key);
	debug!("Received WebSocket ping from {}", conn_key);

	if let Err(e) = state.update_client_ping(conn_key).await {
		record_ws_error!("ping_handling_failed", "websocket", e);
		warn!("Failed to update ping for {}: {}", conn_key, e);
	}

	Ok(())
}

// Handles pong messages
async fn handle_pong_message(state: &WebSocketFsm, conn_key: &str) -> Result<(), ()> {
	record_system_event!("pong_received", connection_id = conn_key);
	debug!("Received WebSocket pong from {}", conn_key);

	if let Err(e) = state.update_client_ping(conn_key).await {
		record_ws_error!("pong_handling_failed", "websocket", e);
		warn!("Failed to update pong for {}: {}", conn_key, e);
	}

	Ok(())
}

// Handles close messages
async fn handle_close_message(state: &WebSocketFsm, conn_key: &str, reason: Option<CloseFrame<'_>>) -> Result<(), ()> {
	record_system_event!("close_received", connection_id = conn_key, reason = reason);
	state.remove_connection(&conn_key, "WebSocket closed".into()).await.ok();
	info!("Client {} closed connection: {:?}", conn_key, reason);
	Err(()) // Signal to break the message processing loop
}

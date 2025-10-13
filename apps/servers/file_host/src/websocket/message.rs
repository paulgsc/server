use super::*;
use crate::utils::retry::retry_async;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::{SplitStream, StreamExt};
use obs_websocket::ObsCommand;
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_connection::ConnectionId;

/// Result of processing a message
#[derive(Debug, Clone)]
pub struct ProcessResult {
	pub delivered: usize,
	pub failed: usize,
	pub duration: Duration,
}

impl Default for ProcessResult {
	fn default() -> Self {
		Self {
			delivered: 0,
			failed: 0,
			duration: Duration::ZERO,
		}
	}
}

impl ProcessResult {
	pub fn success(delivered: usize, duration: Duration) -> Self {
		Self { delivered, failed: 0, duration }
	}

	pub fn failure(failed: usize, duration: Duration) -> Self {
		Self { delivered: 0, failed, duration }
	}
}

impl WebSocketFsm {
	/// Process a text message from a client
	pub async fn process_message(&self, conn_key: &str, raw_message: String) {
		let start = Instant::now();

		// Get connection info
		let connection_id = match self.store.get(conn_key) {
			Some(handle) => handle.connection.id.clone(),
			None => {
				record_ws_error!("connection_not_found", "message_processing");
				error!("Cannot process message for unknown client: {}", conn_key);
				return;
			}
		};

		let message_id = MessageId::new();

		// Parse the message
		let event = match serde_json::from_str::<Event>(&raw_message) {
			Ok(event) => event,
			Err(e) => {
				let duration = start.elapsed();
				record_ws_error!("parse_error", "message", e);
				self.metrics.message_processed(false);
				self.send_error_to_client(conn_key, &format!("Invalid JSON: {}", e)).await;

				// Emit system event
				self
					.record_message_processed(message_id, connection_id, duration, ProcessResult::failure(1, duration))
					.await;
				return;
			}
		};

		// Handle the event based on its type
		let result = match event {
			// Control messages - handled immediately, not broadcast
			Event::Pong => {
				self.handle_pong(conn_key).await;
				ProcessResult::success(1, start.elapsed())
			}

			Event::Subscribe { event_types } => {
				self.handle_subscription(conn_key, event_types, true).await;
				ProcessResult::success(1, start.elapsed())
			}

			Event::Unsubscribe { event_types } => {
				self.handle_subscription(conn_key, event_types, false).await;
				ProcessResult::success(1, start.elapsed())
			}

			// OBS commands - handled asynchronously
			Event::ObsCmd { cmd } => {
				let conn_id = connection_id.clone();
				self.handle_obs_command_async(cmd, conn_id).await;
				ProcessResult::success(1, start.elapsed())
			}

			// All other events - broadcast to subscribers
			_ => self.broadcast_event(&event).await,
		};

		let duration = start.elapsed();
		let success = result.failed == 0;

		// Update metrics
		self.metrics.message_processed(success);

		// Emit system event for observability
		self.record_message_processed(message_id, connection_id, duration, result).await;
	}

	/// Handle pong message (heartbeat response)
	async fn handle_pong(&self, conn_key: &str) {
		if let Err(e) = self.update_client_ping(conn_key).await {
			record_ws_error!("ping_update_failed", "connection", e);
			warn!("Failed to update ping for {}: {}", conn_key, e);
		}
	}

	/// Handle OBS command asynchronously with retry logic
	async fn handle_obs_command_async(&self, cmd: ObsCommand, connection_id: ConnectionId) {
		let obs_manager = self.obs_manager.clone();

		tokio::spawn(async move {
			let cmd_display = format!("{:?}", cmd);
			let start = Instant::now();

			let result = retry_async(
				|| obs_manager.execute_command(cmd.clone()),
				3,                          // max attempts
				Duration::from_millis(100), // base backoff
				2,                          // exponential factor
			)
			.await;

			let duration = start.elapsed();

			match result {
				Ok(_) => {
					record_system_event!(
						"obs_command_success",
						connection_id = connection_id,
						command = cmd_display,
						duration_ms = duration.as_millis()
					);
					info!(
						connection_id = %connection_id,
						command = %cmd_display,
						duration_ms = duration.as_millis(),
						"OBS command executed successfully"
					);
				}
				Err(e) => {
					record_ws_error!("obs_command_failed", "command_execution", &e);
					record_system_event!(
						"obs_command_failed",
						connection_id = connection_id,
						command = cmd_display,
						error = e.to_string(),
						duration_ms = duration.as_millis()
					);
					error!(
						connection_id = %connection_id,
						command = %cmd_display,
						error = %e,
						duration_ms = duration.as_millis(),
						"OBS command failed after retries"
					);
				}
			}
		});
	}
}

/// Process all incoming messages from the WebSocket
pub(crate) async fn process_incoming_messages(mut receiver: SplitStream<WebSocket>, state: &WebSocketFsm, conn_key: &str, cancel_token: CancellationToken) -> u64 {
	let mut message_count = 0u64;

	loop {
		tokio::select! {
			// Listen for cancellation signal
			_ = cancel_token.cancelled() => {
				tracing::info!(
					connection_id = %conn_key,
					messages_processed = message_count,
					"WebSocket message processing cancelled - shutting down"
				);
				break;
			}

			// Process incoming messages
			result = receiver.next() => {
				match result {
					Some(Ok(msg)) => {
						message_count += 1;
						// Handle the message; break on close
						if handle_websocket_message(msg, state, conn_key, message_count).await.is_err() {
							break;
						}
					}
					Some(Err(e)) => {
						message_count += 1;
						record_ws_error!("websocket_error", "connection", e);
						error!(
							connection_id = %conn_key,
							message_number = message_count,
							error = %e,
							"WebSocket error"
						);
						break;
					}
					None => {
						// Stream ended naturally
						tracing::debug!(
							connection_id = %conn_key,
							"WebSocket stream ended"
						);
						break;
					}
				}
			}
		}
	}

	message_count
}

/// Handle a single WebSocket message based on its type
async fn handle_websocket_message(msg: Message, state: &WebSocketFsm, conn_key: &str, message_count: u64) -> Result<(), ()> {
	match msg {
		Message::Text(text) => {
			record_system_event!("message_received", connection_id = conn_key, message_number = message_count, size_bytes = text.len());

			debug!(
				connection_id = %conn_key,
				message_number = message_count,
				size_bytes = text.len(),
				text
			);

			// Process the message
			state.process_message(conn_key, text).await;
			Ok(())
		}

		Message::Ping(_) => {
			record_system_event!("ping_received", connection_id = conn_key);
			debug!(connection_id = %conn_key, "Received WebSocket ping");

			if let Err(e) = state.update_client_ping(conn_key).await {
				record_ws_error!("ping_handling_failed", "websocket", e);
				warn!(
					connection_id = %conn_key,
					error = %e,
					"Failed to update ping"
				);
			}
			Ok(())
		}

		Message::Pong(_) => {
			record_system_event!("pong_received", connection_id = conn_key);
			debug!(connection_id = %conn_key, "Received WebSocket pong");

			if let Err(e) = state.update_client_ping(conn_key).await {
				record_ws_error!("pong_handling_failed", "websocket", e);
				warn!(
					connection_id = %conn_key,
					error = %e,
					"Failed to update pong"
				);
			}
			Ok(())
		}

		Message::Close(reason) => {
			let reason_str = reason
				.as_ref()
				.map(|f| format!("{}: {}", f.code, f.reason))
				.unwrap_or_else(|| "No reason provided".to_string());

			record_system_event!("close_received", connection_id = conn_key, reason = reason_str);

			info!(
				connection_id = %conn_key,
				reason = %reason_str,
				"Client closed connection"
			);

			// Remove the connection
			let _ = state.remove_connection(conn_key, "WebSocket closed".to_string()).await;

			Err(()) // Signal to break the message processing loop
		}

		Message::Binary(data) => {
			debug!(
				connection_id = %conn_key,
				size_bytes = data.len(),
				"Ignored binary message"
			);
			Ok(())
		}
	}
}

// Re-export for compatibility
pub use ProcessResult as EventMessageResult;

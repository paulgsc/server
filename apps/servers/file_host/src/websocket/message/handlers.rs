use crate::{record_ws_error, WS_ERRORS_TOTAL};
use crate::{ConnectionReceivers, WebSocketFsm};
use axum::extract::ws::{Message, WebSocket};
use futures::stream::{SplitStream, StreamExt};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Process all incoming messages from the WebSocket
pub(crate) async fn process_incoming_messages(
	mut receiver: SplitStream<WebSocket>,
	state: &WebSocketFsm,
	conn_key: &str,
	receivers: &ConnectionReceivers,
	cancel_token: CancellationToken,
) -> u64 {
	let mut message_count = 0u64;

	loop {
		tokio::select! {
			// Listen for cancellation signal
			_ = cancel_token.cancelled() => {
				info!(
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
						if handle_websocket_message(msg, state, conn_key, receivers, message_count).await.is_err() {
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
						debug!(
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
async fn handle_websocket_message(msg: Message, state: &WebSocketFsm, conn_key: &str, receivers: &ConnectionReceivers, message_count: u64) -> Result<(), ()> {
	match msg {
		Message::Text(text) => {
			record_system_event!("message_received", connection_id = conn_key, message_number = message_count, size_bytes = text.len());

			debug!(
				connection_id = %conn_key,
				message_number = message_count,
				size_bytes = text.len(),
				"Received text message"
			);

			// Process the message
			state.process_message(conn_key, text, receivers).await;
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

			// Remove the connection (cleanup handled in remove_connection)
			let _ = state.remove_connection(conn_key, "WebSocket closed".to_string()).await;

			Err(()) // Signal to break the message processing loop
		}

		Message::Binary(data) => {
			debug!(
				connection_id = %conn_key,
				size_bytes = data.len(),
				"Received and ignored binary message"
			);
			Ok(())
		}
	}
}

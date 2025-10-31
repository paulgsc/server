use crate::WebSocketFsm;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::{SplitStream, StreamExt};
use some_transport::NatsTransport;
use tokio::{
	sync::mpsc::UnboundedSender,
	time::{interval, Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_events::events::{Event, UnifiedEvent};

/// Process all incoming messages from the WebSocket
pub(crate) async fn process_incoming_messages(
	mut receiver: SplitStream<WebSocket>,
	state: &WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	ws_tx: UnboundedSender<Event>,
	conn_key: &str,
	cancel_token: CancellationToken,
) -> u64 {
	let mut message_count = 0u64;

	// Heartbeat tracking
	let last_activity = Instant::now();
	let mut stale_check_interval = interval(Duration::from_secs(30));
	let stale_timeout = Duration::from_secs(120);

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

			// Periodic stale connection check
			_ = stale_check_interval.tick() => {
				let inactive_duration = Instant::now().duration_since(last_activity);
				if inactive_duration > stale_timeout {
					warn!(
						connection_id = %conn_key,
						inactive_seconds = inactive_duration.as_secs(),
						"Connection is stale - closing"
					);
					let _ = state.remove_connection(conn_key, "Stale connection - no activity".to_string()).await;
					break;
				}
			}

			// Process incoming messages
			result = receiver.next() => {
				match result {
					Some(Ok(msg)) => {
						message_count += 1;
						// Handle the message; break on close
						if handle_websocket_message(msg, state, transport.clone(), ws_tx.clone(), conn_key, message_count).await.is_err() {
							break;
						}
					}
					Some(Err(e)) => {
						message_count += 1;
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
async fn handle_websocket_message(
	msg: Message,
	state: &WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	ws_tx: UnboundedSender<Event>,
	conn_key: &str,
	message_count: u64,
) -> Result<(), ()> {
	match msg {
		Message::Text(text) => {
			debug!(
				connection_id = %conn_key,
				message_number = message_count,
				size_bytes = text.len(),
				"Received text message"
			);

			// Process the message
			state.process_message(transport, ws_tx, conn_key, text).await;
			Ok(())
		}

		Message::Ping(_) => {
			debug!(connection_id = %conn_key, "Received WebSocket ping");
			Ok(())
		}

		Message::Pong(_) => {
			debug!(connection_id = %conn_key, "Received WebSocket pong");
			Ok(())
		}

		Message::Close(reason) => {
			let reason_str = reason
				.as_ref()
				.map(|f| format!("{}: {}", f.code, f.reason))
				.unwrap_or_else(|| "No reason provided".to_string());

			info!(
				connection_id = %conn_key,
				reason = %reason_str,
				"Client closed connection"
			);

			// Remove the connection (cleanup handled in remove_connection)
			let _ = state.remove_connection(conn_key, "WebSocket closed".to_string()).await;
			Err(())
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

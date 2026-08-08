use crate::websocket::connection::instrument;
use crate::WebSocketFsm;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::{SplitStream, StreamExt};
use some_transport::NatsTransport;
use tokio::{
	sync::mpsc::UnboundedSender,
	task::JoinHandle,
	time::{interval, Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_events::events::{Event, UnifiedEvent};

pub(crate) fn spawn_process_incoming_messages(
	receiver: SplitStream<WebSocket>,
	state: WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	ws_tx: UnboundedSender<Event>,
	conn_key: String,
	cancel_token: CancellationToken,
) -> JoinHandle<(u64, &'static str)> {
	tokio::spawn(async move { process_incoming_messages(receiver, state, transport, ws_tx, conn_key, cancel_token).await })
}

/// Process all incoming messages from the WebSocket.
///
/// Returns `(messages_processed, end_reason)` — `end_reason` is one of
/// `timeout` | `client_disconnect` | `error` | `cleanup`, the same label set
/// `ConnectionCleanup::drop` (`websocket.rs`) records under, decided here
/// because this loop is the one place that actually knows *why* it broke.
async fn process_incoming_messages(
	mut receiver: SplitStream<WebSocket>,
	state: WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	ws_tx: UnboundedSender<Event>,
	conn_key: String,
	cancel_token: CancellationToken,
) -> (u64, &'static str) {
	let mut message_count = 0u64;

	let mut stale_check_interval = interval(Duration::from_secs(30));
	let stale_timeout = crate::websocket::STALE_TIMEOUT;

	let store = state.store.clone();
	let end_reason: &'static str;

	loop {
		tokio::select! {
			_ = cancel_token.cancelled() => {
				info!(
					connection_id = %conn_key,
					messages_processed = message_count,
					"WebSocket message processing cancelled - shutting down"
				);
				end_reason = "cleanup";
				break;
			}

			_ = stale_check_interval.tick() => {
				let Some(handle) = store.get(&conn_key) else {
					debug!(
						connection_id = %conn_key,
						"Connection actor missing during stale check - closing"
					);
					end_reason = "cleanup";
					break;
				};

				match handle.get_state().await {
					Ok(state_snapshot) => {
						let inactive = Instant::now()
							.duration_since(state_snapshot.last_activity);

						if inactive > stale_timeout {
							warn!(
								connection_id = %conn_key,
								inactive_seconds = inactive.as_secs(),
								"Connection is stale - closing"
							);

							let _ = state
								.remove_connection(
									&conn_key,
									"Stale connection - no inbound activity".to_string(),
								)
								.await;
							end_reason = "timeout";
							break;
						}
					}
					Err(e) => {
						warn!(
							connection_id = %conn_key,
							error = ?e,
							"Failed to fetch connection state - closing"
						);
						instrument::record_error("state_fetch_failed", "operation");
						end_reason = "error";
						break;
					}
				}
			}

			result = receiver.next() => {
				match result {
					Some(Ok(msg)) => {
						message_count += 1;

						let Some(handle) = store.get(&conn_key) else {
							debug!(
								connection_id = %conn_key,
								"Connection actor missing on inbound frame"
							);
							end_reason = "cleanup";
							break;
						};

						if let Err(e) = handle.record_activity().await {
							warn!(
								connection_id = %conn_key,
								error = ?e,
								"Failed to record inbound activity - closing"
							);
							instrument::record_error("activity_record_failed", "operation");
							end_reason = "error";
							break;
						}

						// Maintain message handling semantics
						if handle_websocket_message(
							msg,
							&state,
							transport.clone(),
							ws_tx.clone(),
							&conn_key
						)
							.await
								.is_err()
						{
							end_reason = "client_disconnect";
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
						instrument::record_error("stream_error", "operation");
						end_reason = "error";
						break;
					}

					None => {
						debug!(
							connection_id = %conn_key,
							"WebSocket stream ended"
						);
						end_reason = "client_disconnect";
						break;
					}
				}
			}
		}
	}

	(message_count, end_reason)
}

/// Handle a single WebSocket message based on its type
async fn handle_websocket_message(
	msg: Message,
	state: &WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	ws_tx: UnboundedSender<Event>,
	conn_key: &str,
) -> Result<(), ()> {
	match msg {
		Message::Text(text) => {
			instrument::record_message("text");
			// Process the message
			state.process_message(transport, ws_tx, conn_key, text).await;
			Ok(())
		}

		Message::Ping(_) => {
			instrument::record_message("ping");
			Ok(())
		}

		Message::Pong(_) => {
			instrument::record_message("pong");
			Ok(())
		}

		Message::Close(reason) => {
			instrument::record_message("close");
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

		Message::Binary(_) => {
			instrument::record_message("binary");
			Ok(())
		}
	}
}

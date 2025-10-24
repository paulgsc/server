use crate::transport::ConnectionReceivers;
use crate::WebSocketFsm;
use axum::extract::ws::{Message, WebSocket};
use futures::{sink::SinkExt, stream::SplitSink};
use some_transport::TransportError;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Spawns task to forward events from NATS subscriptions to WebSocket
///
/// This task multiplexes all subscribed event types and forwards them
/// to the WebSocket connection.
pub(crate) fn spawn_event_forwarder(
	mut sender: SplitSink<WebSocket, Message>,
	receivers: ConnectionReceivers,
	state: WebSocketFsm,
	conn_key: String,
	cancel_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let mut message_count = 0u64;

		// Get all subscribed event types
		let event_types = receivers.event_types();

		info!(
			connection_id = %conn_key,
			subscriptions = ?event_types,
			"Event forwarder started"
		);

		loop {
			tokio::select! {
				// Listen for cancellation signal
				_ = cancel_token.cancelled() => {
					info!(
						connection_id = %conn_key,
						messages_forwarded = message_count,
						"Event forwarder cancelled - shutting down"
					);
					break;
				}

				// Multiplex all receivers - poll each event type
				result = receive_from_any(&receivers) => {
					match result {
						Ok((event_type, event)) => {
							message_count += 1;

							if let Err(e) = forward_event(&mut sender, &event, &conn_key).await {
								error!(
									connection_id = %conn_key,
									event_type = ?event_type,
									error = %e,
									"Error forwarding event"
								);
								break; // Fatal: stop forwarding
							}

							if message_count % 100 == 0 {
								record_system_event!(
									"forward_milestone",
									connection_id = conn_key,
									messages_forwarded = message_count
								);
								debug!(
									connection_id = %conn_key,
									messages_forwarded = message_count,
									"Forwarding milestone reached"
								);
							}
						}
						Err(ForwardError::Lagged(event_type, n)) => {
							warn!(
								connection_id = %conn_key,
								event_type = ?event_type,
								skipped = n,
								"Receiver lagged, skipped messages"
							);
							record_ws_error!("receiver_overflow", "forward", n);
							continue;
						}
						Err(ForwardError::Closed(event_type)) => {
							warn!(
								connection_id = %conn_key,
								event_type = ?event_type,
								"Event channel closed"
							);
							// One channel closed, but continue with others
							continue;
						}
						Err(ForwardError::NoReceivers) => {
							debug!(
								connection_id = %conn_key,
								"No active receivers, ending forwarder"
							);
							break;
						}
					}
				}
			}
		}

		// Cleanup: remove connection from store
		let _ = state.remove_connection(&conn_key, "Event forwarder ended".to_string()).await;

		record_system_event!("forward_ended", connection_id = conn_key, total_messages = message_count);
		debug!(
			connection_id = %conn_key,
			total_messages = message_count,
			"Event forwarding ended"
		);
	})
}

/// Receive from any of the connection's receivers using tokio::select!
async fn receive_from_any(receivers: &ConnectionReceivers) -> Result<(EventType, Event), ForwardError> {
	use tokio::select;

	// Get clones of all receivers
	let mut system_rx = receivers.get(&EventType::System);
	let mut audio_rx = receivers.get(&EventType::Audio);
	let mut chat_rx = receivers.get(&EventType::Chat);
	let mut obs_rx = receivers.get(&EventType::Obs);
	let mut client_count_rx = receivers.get(&EventType::ClientCount);
	let mut error_rx = receivers.get(&EventType::Error);

	// If no receivers, return error
	if system_rx.is_none() && audio_rx.is_none() && chat_rx.is_none() && obs_rx.is_none() && client_count_rx.is_none() && error_rx.is_none() {
		return Err(ForwardError::NoReceivers);
	}

	// Use select! to wait for first available message
	select! {
		result = async {
			if let Some(ref mut rx) = system_rx {
				rx.recv().await
			} else {
				std::future::pending().await
			}
		} => {
			handle_receive_result(result, EventType::System)
		}

		result = async {
			if let Some(ref mut rx) = audio_rx {
				rx.recv().await
			} else {
				std::future::pending().await
			}
		} => {
			handle_receive_result(result, EventType::Audio)
		}

		result = async {
			if let Some(ref mut rx) = chat_rx {
				rx.recv().await
			} else {
				std::future::pending().await
			}
		} => {
			handle_receive_result(result, EventType::Chat)
		}

		result = async {
			if let Some(ref mut rx) = obs_rx {
				rx.recv().await
			} else {
				std::future::pending().await
			}
		} => {
			handle_receive_result(result, EventType::Obs)
		}

		result = async {
			if let Some(ref mut rx) = client_count_rx {
				rx.recv().await
			} else {
				std::future::pending().await
			}
		} => {
			handle_receive_result(result, EventType::ClientCount)
		}

		result = async {
			if let Some(ref mut rx) = error_rx {
				rx.recv().await
			} else {
				std::future::pending().await
			}
		} => {
			handle_receive_result(result, EventType::Error)
		}
	}
}

/// Handle the result of receiving from a transport receiver
fn handle_receive_result(result: Result<Event, TransportError>, event_type: EventType) -> Result<(EventType, Event), ForwardError> {
	match result {
		Ok(event) => Ok((event_type, event)),
		Err(TransportError::Overflowed(n)) => Err(ForwardError::Lagged(event_type, n)),
		Err(TransportError::Closed) => Err(ForwardError::Closed(event_type)),
		Err(e) => {
			error!("Transport error on {:?}: {}", event_type, e);
			Err(ForwardError::Closed(event_type))
		}
	}
}

/// Forward a single event to the WebSocket client
async fn forward_event(sender: &mut SplitSink<WebSocket, Message>, event: &Event, conn_key: &str) -> Result<(), String> {
	// Serialize event
	let json = serde_json::to_string(event).map_err(|e| {
		let msg = format!("Failed to serialize event for {}: {}", conn_key, e);
		record_ws_error!("serialization_failed", "forward", &msg);
		msg
	})?;

	let msg = Message::Text(json);

	// Send message
	sender.send(msg).await.map_err(|e| {
		let msg = format!("Failed to forward event to {}: {}", conn_key, e);
		record_ws_error!("forward_send_failed", "forward", &msg);
		msg
	})?;

	Ok(())
}

use super::errors::BroadcastError;
use crate::WebSocketFsm;
use axum::extract::ws::{Message, WebSocket};
use futures::{sink::SinkExt, stream::SplitSink};
use some_transport::{NatsTransport, NatsTransportReceiver, Transport, TransportError};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_events::{
	events::{Event, EventType},
	UnifiedEvent,
};

/// Spawns task to forward events from shared static NATS subscriptions to WebSocket
///
/// This task multiplexes from the app-wide static receiver set and filters events
/// based on the connection's active subscriptions stored in their handle.
pub(crate) fn spawn_event_forwarder(
	mut sender: SplitSink<WebSocket, Message>,
	state: WebSocketFsm,
	conn_key: String,
	cancel_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let mut message_count = 0u64;
		let mut filtered_count = 0u64;

		let transport = state.transport;
		loop {
			tokio::select! {
				// Listen for cancellation signal
				_ = cancel_token.cancelled() => {
					info!(
						connection_id = %conn_key,
						messages_forwarded = message_count,
						messages_filtered = filtered_count,
						"Event forwarder cancelled - shutting down"
					);
					break;
				}

				// Multiplex from shared receivers - ALL event types are polled
				result = receive_from_any(transport) => {
					match result {
						Ok((event_type, event)) => {
							// Check if THIS connection is subscribed to this event type
							let is_subscribed = if let Some(handle) = state.store.get(&conn_key) {
								match handle.is_subscribed_to(event_type).await {
									Ok(subscribed) => subscribed,
									Err(e) => {
										warn!(
											connection_id = %conn_key,
											error = %e,
											"Failed to check subscription state"
										);
										break;
									}
								}
							} else {
								// Connection no longer exists in store
								debug!(
									connection_id = %conn_key,
									"Connection not found in store, stopping forwarder"
								);
								break;
							};

							if is_subscribed {
								// Forward event to this connection
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
									debug!(
										connection_id = %conn_key,
										messages_forwarded = message_count,
										messages_filtered = filtered_count,
										"Forwarding milestone reached"
									);
								}
							} else {
								// Event received but this connection not subscribed - filter it out
								filtered_count += 1;

								if filtered_count % 1000 == 0 {
									debug!(
										connection_id = %conn_key,
										event_type = ?event_type,
										total_filtered = filtered_count,
										"Filtering events not in subscription set"
									);
								}
							}
						}
						Err(BroadcastError::Lagged(event_type, n)) => {
							warn!(
								connection_id = %conn_key,
								event_type = ?event_type,
								skipped = n,
								"Receiver lagged, skipped messages"
							);
							continue;
						}
						Err(BroadcastError::Closed(event_type)) => {
							warn!(
								connection_id = %conn_key,
								event_type = ?event_type,
								"Event channel closed in shared receivers"
							);
							// One channel closed, but continue with others
							continue;
						}
						Err(BroadcastError::NoReceivers) => {
							error!(
								connection_id = %conn_key,
								"Shared receiver set is empty - critical error"
							);
							break;
						}
					}
				}
			}
		}

		// Cleanup: remove connection from store
		let _ = state.remove_connection(&conn_key, "Event forwarder ended".to_string()).await;

		info!(
			connection_id = %conn_key,
			total_messages_forwarded = message_count,
			total_messages_filtered = filtered_count,
			"Event forwarding ended"
		);
	})
}

/// Receive from any receiver in the shared static set using tokio::select!
///
/// This multiplexes ALL available event types. Every connection polls the same
/// shared receivers, but filtering happens per-connection based on their subscription state.
async fn receive_from_any(transport: NatsTransport<UnifiedEvent>) -> Result<(EventType, Event), BroadcastError> {
	// Per-connection NATS receivers for each subject
	let mut ping_rx = transport.subscribe_to_subject(&EventType::Ping.subject()).await;
	let mut pong_rx = transport.subscribe_to_subject(&EventType::Pong.subject()).await;
	let mut error_rx = transport.subscribe_to_subject(&EventType::Error.subject()).await;
	let mut client_count_rx = transport.subscribe_to_subject(&EventType::ClientCount.subject()).await;
	let mut obs_command_rx = transport.subscribe_to_subject(&EventType::ObsCommand.subject()).await;
	let mut obs_status_rx = transport.subscribe_to_subject(&EventType::ObsStatus.subject()).await;
	let mut tab_meta_rx = transport.subscribe_to_subject(&EventType::TabMetaData.subject()).await;
	let mut utterance_rx = transport.subscribe_to_subject(&EventType::Utterance.subject()).await;
	let mut conn_state_rx = transport.subscribe_to_subject(&EventType::ConnectionStateChanged.subject()).await;
	let mut msg_processed_rx = transport.subscribe_to_subject(&EventType::MessageProcessed.subject()).await;
	let mut broadcast_failed_rx = transport.subscribe_to_subject(&EventType::BroadcastFailed.subject()).await;
	let mut conn_cleanup_rx = transport.subscribe_to_subject(&EventType::ConnectionCleanup.subject()).await;

	// Multiplex all receivers - whichever fires first wins
	select! {
		result = recv_or_pending(&mut ping_rx) => {
			handle_receive_result(result, EventType::Ping)
		}

		result = recv_or_pending(&mut pong_rx) => {
			handle_receive_result(result, EventType::Pong)
		}

		result = recv_or_pending(&mut error_rx) => {
			handle_receive_result(result, EventType::Error)
		}

		result = recv_or_pending(&mut client_count_rx) => {
			handle_receive_result(result, EventType::ClientCount)
		}

		result = recv_or_pending(&mut obs_command_rx) => {
			handle_receive_result(result, EventType::ObsCommand)
		}

		result = recv_or_pending(&mut obs_status_rx) => {
			handle_receive_result(result, EventType::ObsStatus)
		}

		result = recv_or_pending(&mut tab_meta_rx) => {
			handle_receive_result(result, EventType::TabMetaData)
		}

		result = recv_or_pending(&mut utterance_rx) => {
			handle_receive_result(result, EventType::Utterance)
		}

		result = recv_or_pending(&mut conn_state_rx) => {
			handle_receive_result(result, EventType::ConnectionStateChanged)
		}

		result = recv_or_pending(&mut msg_processed_rx) => {
			handle_receive_result(result, EventType::MessageProcessed)
		}

		result = recv_or_pending(&mut broadcast_failed_rx) => {
			handle_receive_result(result, EventType::BroadcastFailed)
		}

		result = recv_or_pending(&mut conn_cleanup_rx) => {
			handle_receive_result(result, EventType::ConnectionCleanup)
		}
	}
}

/// Helper to recv from a receiver or return pending future if None
async fn recv_or_pending(rx: &mut NatsTransportReceiver<UnifiedEvent>) -> Result<UnifiedEvent, TransportError> {
	rx.recv().await
}

/// Handle the result of receiving from a transport receiver
fn handle_receive_result(result: Result<UnifiedEvent, TransportError>, event_type: EventType) -> Result<(EventType, Event), BroadcastError> {
	match result {
		Ok(unified) => {
			let event: Event = Result::<Event, String>::from(unified).map_err(|e| {
				error!("💥 UnifiedEvent conversion failed for {:?}: {}", event_type.clone(), e);
				BroadcastError::Closed(event_type.clone())
			})?;
			Ok((event_type, event))
		}
		Err(TransportError::Overflowed(n)) => Err(BroadcastError::Lagged(event_type, n)),
		Err(TransportError::Closed) => Err(BroadcastError::Closed(event_type)),
		Err(e) => {
			error!("Transport error on {:?}: {}", event_type, e);
			Err(BroadcastError::Closed(event_type))
		}
	}
}

/// Forward a single event to the WebSocket client
async fn forward_event(sender: &mut SplitSink<WebSocket, Message>, event: &Event, conn_key: &str) -> Result<(), String> {
	// Serialize event
	let json = serde_json::to_string(event).map_err(|e| {
		let msg = format!("Failed to serialize event for {}: {}", conn_key, e);
		msg
	})?;

	let msg = Message::Text(json);

	// Send message
	sender.send(msg).await.map_err(|e| {
		let msg = format!("Failed to forward event to {}: {}", conn_key, e);
		msg
	})?;

	Ok(())
}

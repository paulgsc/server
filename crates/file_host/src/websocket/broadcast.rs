use super::*;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::SplitSink;
use some_transport::{InMemTransportReceiver, Transport, TransportError};
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tracing::{debug, error, warn};

type R = InMemTransportReceiver<Event>;
type T = Arc<InMemTransport<Event>>;

/// Pure broadcast result without side effects
#[derive(Debug)]
pub struct BroadcastResult {
	pub delivered: usize,
	pub failed: usize,
	pub duration: Duration,
}

impl BroadcastResult {
	fn success(count: usize, duration: Duration) -> Self {
		Self {
			delivered: count,
			failed: 0,
			duration,
		}
	}

	fn no_subscribers() -> Self {
		Self {
			delivered: 0,
			failed: 0,
			duration: Duration::default(),
		}
	}
}

impl WebSocketFsm {
	/// Spawns the event distribution task
	pub fn spawn_event_distribution_task(&self) {
		let transport = self.transport.clone();
		let metrics = self.metrics.clone();

		tokio::spawn(async move {
			let mut receiver = transport.subscribe();

			loop {
				match receiver.recv().await {
					Ok(event) => {
						if event.is_client_event() {
							let event_type_str = format!("{:?}", event.get_type().unwrap_or_default());

							// Pure broadcast + telemetry in one place
							let result = timed_broadcast!(&event_type_str, { Self::broadcast_pure(&event, transport.clone()).await });

							// Record metrics
							match result {
								Ok(br) if br.delivered > 0 || br.failed > 0 => {
									metrics.broadcast_attempt(br.failed == 0);
								}
								Err(e) => {
									record_ws_error!("broadcast_failed", "main_channel", &e);
									metrics.broadcast_attempt(false);
									warn!("Broadcast failed: {}", e);
								}
								_ => {}
							}
						}
					}
					Err(TransportError::Closed) => {
						record_ws_error!("channel_closed", "main_receiver");
						break;
					}
					Err(TransportError::Overflowed(count)) => {
						record_ws_error!("channel_overflow", "main_receiver");
						warn!("Main receiver lagged behind by {} messages", count);
					}
					Err(e) => {
						error!("Transport error in distribution loop: {}", e);
					}
				}
			}
		});
	}

	/// Pure broadcast function - no side effects
	pub async fn broadcast_pure(event: &Event, transport: T) -> Result<BroadcastResult, TransportError> {
		if !event.is_client_event() {
			return Ok(BroadcastResult::no_subscribers());
		}

		let start = Instant::now();
		let result = transport.broadcast(event.clone()).await;
		let duration = start.elapsed();

		match result {
			Ok(count) => Ok(BroadcastResult::success(count, duration)),
			Err(e) => Err(e),
		}
	}

	/// Public API method with full telemetry and system events
	pub async fn broadcast_event(&self, event: &Event) -> ProcessResult {
		let result = Self::broadcast_pure(event, self.transport.clone()).await;

		match result {
			Ok(br) => {
				if br.delivered > 0 || br.failed > 0 {
					self.metrics.broadcast_attempt(br.failed == 0);
				}
				ProcessResult {
					delivered: br.delivered,
					failed: br.failed,
					duration: br.duration,
				}
			}
			Err(e) => {
				record_ws_error!("broadcast_failed", "main_channel", &e);
				self.metrics.broadcast_attempt(false);

				// Emit system event for monitoring
				if let Some(event_type) = event.get_type() {
					self
						.emit_system_event(Event::BroadcastFailed {
							event_type,
							error: e.to_string(),
							affected_connections: 0,
						})
						.await;
				}

				ProcessResult {
					delivered: 0,
					failed: 1,
					duration: Duration::default(),
				}
			}
		}
	}

	pub(crate) async fn broadcast_client_count(&self) {
		let count = self.store.len();
		let event = Event::ClientCount { count };
		let _ = self.broadcast_event(&event).await;
		update_resource_usage!("active_connections", count as f64);
	}
}

// Spawns task to forward events from broadcast channel to WebSocket
pub(crate) fn spawn_event_forwarder(mut sender: SplitSink<WebSocket, Message>, mut event_receiver: R, state: WebSocketFsm, conn_key: String) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let mut message_count = 0u64;

		loop {
			match event_receiver.recv().await {
				Ok(event) => {
					// CRITICAL: Filter out system events - only forward client events
					if !event.is_client_event() {
						continue;
					}

					message_count += 1;

					if let Err(e) = forward_single_event(&mut sender, &event, &conn_key, message_count).await {
						record_ws_error!("forward_single_event_error", "forward", e);
						error!(
							connection_id = %conn_key,
							error = %e,
							"Error forwarding single event"
						);
						break; // Fatal: stop forwarding
					}

					if message_count % 100 == 0 {
						record_system_event!("forward_milestone", connection_id = conn_key, messages_forwarded = message_count);
						debug!(
							connection_id = %conn_key,
							messages_forwarded = message_count,
							"Forwarding milestone reached"
						);
					}
				}

				Err(TransportError::Overflowed(n)) => {
					// Client is lagging behind — warn, skip, but continue
					warn!(
						skipped = n,
						connection_id = %conn_key,
						"Event receiver lagged, skipped {} messages",
						n
					);
					record_ws_error!("receiver_overflow", "forward", n);
					continue;
				}

				Err(TransportError::Closed) => {
					// Fatal: channel closed → stop loop
					error!(
						connection_id = %conn_key,
						"Event channel closed, ending forwarder"
					);
					record_ws_error!("channel_closed", "forward");
					break;
				}

				Err(e) => {
					error!(
						connection_id = %conn_key,
						error = %e,
						"Transport error in forwarder"
					);
					break;
				}
			}
		}

		// Invariant: receiver gone → connection gone
		let _ = state.remove_connection(&conn_key, "Event forwarder ended".to_string()).await;

		record_system_event!("forward_ended", connection_id = conn_key, total_messages = message_count);
		debug!(
			connection_id = %conn_key,
			total_messages = message_count,
			"Event forwarding ended"
		);
	})
}

// Forwards a single event to the WebSocket client
async fn forward_single_event(sender: &mut SplitSink<WebSocket, Message>, event: &Event, conn_key: &str, message_count: u64) -> Result<(), String> {
	// Serialize event
	let json = timed_ws_operation!("forward", "serialize", { serde_json::to_string(event) }).map_err(|e| {
		let msg = format!("Failed to serialize event for client {}: {}", conn_key, e);
		record_ws_error!("serialization_failed", "forward", &msg);
		error!("{}", msg);
		msg
	})?;

	let msg = Message::Text(json);

	// Send message
	timed_ws_operation!("forward", "send", { sender.send(msg).await }).map_err(|e| {
		let msg = format!("Failed to forward event to client {} (msg #{}): {}", conn_key, message_count, e);
		record_ws_error!("forward_send_failed", "forward", &msg);
		error!("{}", msg);
		msg
	})?;

	Ok(())
}

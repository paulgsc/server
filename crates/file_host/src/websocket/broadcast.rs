use super::*;
use async_broadcast::{Receiver, RecvError};
use axum::extract::ws::{Message, WebSocket};
use futures::stream::SplitSink;
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tracing::{debug, error, warn};

pub enum BroadcastOutcome {
	NoSubscribers,
	Completed { process_result: ProcessResult },
}

impl WebSocketFsm {
	/// Spawns the event distribution task - single responsibility
	pub fn spawn_event_distribution_task(&self) {
		let transport = self.transport.clone();
		let store = self.store.clone();
		let metrics = self.metrics.clone();

		tokio::spawn(async move {
			Self::event_distribution_loop(transport, store, metrics).await;
		});
	}

	/// Event distribution loop - isolated logic
	async fn event_distribution_loop(transport: Arc<TransportLayer>, store: Arc<ConnectionStore<EventType>>, metrics: Arc<ConnectionMetrics>) {
		let mut receiver = transport.main_sender().new_receiver();

		loop {
			match receiver.recv().await {
				Ok(event) => {
					Self::handle_event_broadcast(event, store.clone(), &metrics).await;
				}
				Err(e) => {
					if Self::handle_receiver_error(e) {
						break; // Exit loop on closed channel
					}
					// Continue on overflow
				}
			}
		}
	}

	/// Handles broadcasting a single event
	async fn handle_event_broadcast(event: Event, store: Arc<ConnectionStore<EventType>>, metrics: &Arc<ConnectionMetrics>) {
		let event_type = event.get_type();
		let event_type_str = format!("{:?}", event_type);

		let broadcast_outcome: Result<BroadcastOutcome, String> = timed_broadcast!(&event_type_str, { Ok(Self::broadcast_event_to_subscribers(event, &event_type, store).await) });

		Self::handle_broadcast_outcome(broadcast_outcome, metrics);
	}

	/// Handles the result of a broadcast operation
	fn handle_broadcast_outcome(broadcast_outcome: Result<BroadcastOutcome, String>, metrics: &Arc<ConnectionMetrics>) {
		match broadcast_outcome {
			Ok(broadcast_outcome) => match broadcast_outcome {
				BroadcastOutcome::NoSubscribers => {
					// Nothing to do
				}
				BroadcastOutcome::Completed {
					process_result: ProcessResult { failed, .. },
				} => {
					metrics.broadcast_attempt(failed == 0);
				}
			},
			Err(_) => {
				record_ws_error!("channel_closed", "main_receiver");
			}
		}
	}

	/// Handles receiver errors, returns true if should exit loop
	fn handle_receiver_error(error: RecvError) -> bool {
		match error {
			RecvError::Closed => {
				record_ws_error!("channel_closed", "main_receiver", error);
				true // Exit the loop
			}
			RecvError::Overflowed(count) => {
				record_ws_error!("channel_overflow", "main_receiver");
				warn!("Main receiver lagged behind by {} messages, continuing", count);
				false // Continue processing
			}
		}
	}

	/// Broadcasts an event to all subscribed and active connections
	pub(crate) async fn broadcast_event_to_subscribers(event: Event, event_type: &EventType, store: Arc<ConnectionStore<EventType>>) -> BroadcastOutcome {
		let start_time = Instant::now();
		let mut delivered = 0;
		let mut failed = 0;

		// Collect handles for subscribed connections
		let subscribed_handles: Vec<_> = {
			let keys = store.keys();
			let mut handles = Vec::new();

			for key in keys {
				if let Some(handle) = store.get(&key) {
					// Check subscription (immutable, no async needed)
					if handle.is_subscribed_to(event_type) {
						// Check if active via actor state
						if let Ok(state) = handle.get_state().await {
							if state.is_active {
								handles.push((key, handle));
							}
						}
					}
				}
			}
			handles
		};

		if subscribed_handles.is_empty() {
			return BroadcastOutcome::NoSubscribers;
		}

		// Send to each subscribed connection via transport
		for (conn_key, handle) in subscribed_handles {
			// Note: We can't use handle.connection.sender anymore since it's removed
			// Instead, we rely on the per-connection transport channel
			// This is a design decision - events go through transport layer
			match self.transport.send_to_connection_transport(&conn_key, event.clone()).await {
				Ok(_) => delivered += 1,
				Err(e) => {
					failed += 1;
					record_ws_error!("send_failed", "broadcast", e);
					warn!("Failed to send event {:?} to client {}: {}", event_type, handle.connection.id, e);
				}
			}
		}

		let duration = start_time.elapsed();
		let process_result = ProcessResult { delivered, failed, duration };

		BroadcastOutcome::Completed { process_result }
	}

	pub async fn broadcast_event(&self, event: &Event) -> ProcessResult {
		let event_type_str = format!("{:?}", event.get_type());
		let receiver_count = self.sender.receiver_count();

		if self.sender.receiver_count() == 0 {
			return ProcessResult {
				delivered: 0,
				failed: 0,
				duration: Duration::default(),
			};
		}

		let result = timed_broadcast!(&event_type_str, {
			match self.sender.broadcast(event.clone()).await {
				Ok(_) => Ok(BroadcastOutcome::Completed {
					process_result: ProcessResult {
						delivered: receiver_count,
						failed: 0,
						duration: Duration::default(),
					},
				}),
				Err(e) => {
					record_ws_error!("broadcast_failed", "main_channel", e);
					self.metrics.broadcast_attempt(false);

					// Emit system event for monitoring
					record_system_event!("broadcast_failed", event_type = event.get_type(), error = e, affected_connections = receiver_count);

					Err(format!("Broadcast failed: {}", e))
				}
			}
		});

		match result {
			Ok(broadcast_outcome) => match broadcast_outcome {
				BroadcastOutcome::Completed { process_result } => process_result,
				BroadcastOutcome::NoSubscribers => ProcessResult {
					delivered: 0,
					failed: 0,
					duration: Duration::default(),
				},
			},
			Err(_) => ProcessResult {
				delivered: 0,
				failed: 1,
				duration: Duration::default(),
			},
		}
	}

	pub(crate) async fn broadcast_client_count(&self) {
		let count = self.connections.len();
		let _ = self.sender.broadcast(Event::ClientCount { count }).await;
		update_resource_usage!("active_connections", count as f64);
	}
}

// Spawns task to forward events from broadcast channel to WebSocket

pub(crate) fn spawn_event_forwarder(
	mut sender: SplitSink<WebSocket, Message>,
	mut event_receiver: Receiver<Event>,
	state: WebSocketFsm,
	conn_key: String,
) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let mut message_count = 0u64;

		loop {
			match event_receiver.recv().await {
				Ok(event) => {
					message_count += 1;

					if let Err(e) = forward_single_event(&mut sender, &event, &conn_key, message_count).await {
						record_ws_error!("forward single event error", "forward");
						error!("Error forwarding single event: {:?}", e);
						break; // Fatal: stop forwarding
					}

					if message_count % 100 == 0 {
						record_system_event!("forward_milestone", connection_id = conn_key, messages_forwarded = message_count);
						debug!("Forwarded {} messages to client {}", message_count, conn_key);
					}
				}

				Err(RecvError::Overflowed(n)) => {
					// Client is lagging behind — warn, skip, but continue
					warn!(skipped = n, connection_id = conn_key, "Event receiver lagged, skipped {} messages", n);
					record_ws_error!("Event receiver lagged, skipped {} messages", "forward", n);
					continue;
				}

				Err(RecvError::Closed) => {
					// Fatal: channel closed → stop loop
					error!(connection_id = conn_key, "Event channel closed, ending forwarder");
					record_ws_error!("Event channel closed unexpectedly", "forward");
					break;
				}
			}
		}

		// Invariant: receiver gone → connection gone
		let _ = state.remove_connection(&conn_key, "Event forwarder ended".to_string()).await;
		record_system_event!("forward_ended", connection_id = conn_key, total_messages = message_count);
		debug!("Event forwarding ended for client {} after {} messages", conn_key, message_count);
	})
}

// Forwards a single event to the WebSocket client
async fn forward_single_event(sender: &mut SplitSink<WebSocket, Message>, event: &Event, conn_key: &str, message_count: u64) -> Result<(), ()> {
	let result = timed_ws_operation!("forward", "serialize", { serde_json::to_string(event) });

	let msg = match result {
		Ok(json) => Message::Text(json),
		Err(e) => {
			record_ws_error!("serialization_failed", "forward", e);
			error!("Failed to serialize event for client {}: {}", conn_key, e);
			return Err(());
		}
	};

	let send_result = timed_ws_operation!("forward", "send", { sender.send(msg).await });

	if let Err(e) = send_result {
		record_ws_error!("forward_send_failed", "forward", e);
		error!("Failed to forward event to client {} (msg #{}): {}", conn_key, message_count, e);
		return Err(());
	}

	Ok(())
}

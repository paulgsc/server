use crate::*;
use async_broadcast::{broadcast, Receiver, Sender};
use axum::{
	extract::{
		ws::{Message, WebSocket, WebSocketUpgrade},
		State,
	},
	response::IntoResponse,
	routing::get,
	Router,
};
use dashmap::DashMap;
use futures::{sink::SinkExt, stream::StreamExt};
use serde::Serialize;
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

pub mod broadcast;
pub mod connection;
pub mod message;
pub mod types;

pub use broadcast::BroadcastOutcome;
pub use connection::Connection;
pub use message::{EventMessage, MessageState, ProcessResult};
pub use types::*;

// Enhanced WebSocket FSM with comprehensive observability
#[derive(Clone)]
pub struct WebSocketFsm {
	connections: Arc<DashMap<String, Connection>>,
	sender: Sender<Event>,
	metrics: Arc<ConnectionMetrics>,
	system_events: Sender<SystemEvent>,
}

impl WebSocketFsm {
	pub fn new() -> Self {
		let (mut sender, receiver) = broadcast::<Event>(1000); // Larger buffer for main channel
		sender.set_await_active(false);
		sender.set_overflow(true);

		let (system_sender, _system_receiver) = broadcast::<SystemEvent>(500);

		let connections = Arc::new(DashMap::<String, Connection>::new());
		let metrics = Arc::new(ConnectionMetrics::default());

		// Event distribution task
		let conn_fan = connections.clone();
		let metrics_clone = metrics.clone();
		// let system_events_clone = system_sender.clone();

		tokio::spawn(async move {
			let mut receiver = receiver;

			loop {
				match receiver.recv().await {
					Ok(event) => {
						let event_type = event.get_type();
						let event_type_str = format!("{:?}", event_type);

						let broadcast_outcome: Result<BroadcastOutcome, String> =
							timed_broadcast!(&event_type_str, { Ok(Self::broadcast_event_to_subscribers(&conn_fan, event, &event_type).await) });

						match broadcast_outcome {
							Ok(broadcast_outcome) => match broadcast_outcome {
								BroadcastOutcome::NoSubscribers => continue,
								BroadcastOutcome::Completed {
									process_result: ProcessResult { failed, .. },
								} => {
									metrics_clone.broadcast_attempt(failed == 0);
								}
							},
							Err(_) => {
								record_ws_error!("channel_closed", "main_receiver");
							}
						}
					}
					Err(e) => match e {
						async_broadcast::RecvError::Closed => {
							record_ws_error!("channel_closed", "main_receiver", e);
							break;
						}
						async_broadcast::RecvError::Overflowed(count) => {
							record_ws_error!("channel_overflow", "main_receiver");
							warn!("Main receiver lagged behind by {} messages, continuing", count);
							continue;
						}
					},
				}
			}
		});

		record_system_event!("fsm_initialized");
		update_resource_usage!("active_connections", 0.0);

		Self {
			connections,
			sender,
			metrics,
			system_events: system_sender,
		}
	}

	pub fn router(self) -> Router {
		Router::new().route("/ws", get(websocket_handler)).with_state(self)
	}

	// Enhanced message processing with full traceability
	pub async fn process_message(&self, client_key: &str, raw_message: String) {
		// Get connection ID for correlation
		let connection_id = if let Some(conn) = self.connections.get(client_key) {
			conn.id.clone()
		} else {
			record_ws_error!("connection_not_found", "message_processing");
			error!("Cannot process message for unknown client: {}", client_key);
			return;
		};

		let mut message = EventMessage::new(connection_id.clone(), raw_message);

		// Update message count
		if let Some(mut conn) = self.connections.get_mut(client_key) {
			conn.increment_message_count();
		}

		// Parse
		if let Err(e) = message.parse() {
			record_message_result!("unknown", "parse_failed", connection_id: connection_id);
			record_ws_error!("parse_error", "message", e);
			self.metrics.message_processed(false);
			self.send_error_to_client(client_key, &e).await;
			return;
		}

		// Handle control messages immediately
		if let Some(event) = message.get_event() {
			let event_type_str = format!("{:?}", event.get_type());

			match event {
				Event::Pong => {
					record_message_result!(&event_type_str, "success", connection_id: connection_id);
					if let Err(e) = self.update_client_ping(client_key).await {
						record_ws_error!("ping_update_failed", "connection", e);
					}
					self.metrics.message_processed(true);
					return;
				}
				Event::Subscribe { event_types } => {
					record_message_result!(&event_type_str, "success", connection_id: connection_id);
					self.handle_subscription(client_key, event_types.clone(), true).await;
					self.metrics.message_processed(true);
					return;
				}
				Event::Unsubscribe { event_types } => {
					record_message_result!(&event_type_str, "success", connection_id: connection_id);
					self.handle_subscription(client_key, event_types.clone(), false).await;
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
			self.send_error_to_client(client_key, &e).await;
			return;
		}

		// Process (broadcast)
		if let Some(event) = message.get_event() {
			let event_type_str = format!("{:?}", event.get_type());
			let start_time = Instant::now();

			let result = timed_ws_operation!(&event_type_str, "process", { self.broadcast_event(event).await });

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

	async fn handle_subscription(&self, client_key: &str, event_types: Vec<EventType>, subscribe: bool) {
		if let Some(mut conn) = self.connections.get_mut(client_key) {
			let changed_count = if subscribe {
				conn.subscribe(event_types.clone())
			} else {
				conn.unsubscribe(event_types.clone())
			};

			let operation = if subscribe { "subscribe" } else { "unsubscribe" };
			record_subscription_change!(operation, &event_types, changed_count, conn.id);
		}
	}

	async fn send_error_to_client(&self, client_key: &str, error: &str) {
		if let Some(connection) = self.connections.get(client_key) {
			let error_event = Event::Error { message: error.to_string() };
			if let Err(e) = connection.send_event(error_event).await {
				record_ws_error!("error_send_failed", "connection", e);
				warn!("Failed to send error to client {}: {}", connection.id, e);
			}
		}
	}

	async fn update_client_ping(&self, client_key: &str) -> Result<(), String> {
		if let Some(mut connection) = self.connections.get_mut(client_key) {
			let connection_id = connection.id.clone();
			match connection.update_ping() {
				Ok(old_state) => {
					// Emit state change event if there was a transition
					if !matches!(old_state, ConnectionState::Active { .. }) {
						update_connection_state!("stale", "active");
						record_system_event!(
							"connection_state_changed",
							connection_id = connection_id,
							from_state = old_state,
							to_state = connection.state
						);
					}
					Ok(())
				}
				Err(e) => {
					record_ws_error!("ping_update_failed", "connection", &e);
					Err(format!("Failed to update ping for {}: {}", connection_id, e))
				}
			}
		} else {
			record_ws_error!("ping_update_no_connection", "connection");
			Err(format!("Client {} not found", client_key))
		}
	}

	pub async fn get_client_count(&self) -> usize {
		self.connections.len()
	}

	pub fn get_metrics(&self) -> ConnectionMetricsSnapshot {
		self.metrics.get_snapshot()
	}

	pub fn subscribe_to_system_events(&self) -> Receiver<SystemEvent> {
		self.system_events.new_receiver()
	}

	// Health check endpoint data
	pub async fn get_health_status(&self) -> HealthStatus {
		let health_result: Result<HealthStatus, ()> = health_check!("health_status", {
			let metrics = self.get_metrics();
			let connection_states = self.get_connection_state_distribution().await;

			// Check system invariants
			check_invariant!(!self.sender.is_closed(), "sender_state", "Main sender channel is closed");

			check_invariant!(
				self.sender.receiver_count() > 0 || self.connections.is_empty(),
				"receiver_count",
				"No receivers but connections exist",
				expected: "receivers > 0 or connections == 0",
				actual: format!("receivers: {}, connections: {}", self.sender.receiver_count(), self.connections.len())
			);

			Ok(HealthStatus {
				total_connections: self.connections.len(),
				metrics,
				connection_states,
				sender_receiver_count: self.sender.receiver_count(),
				sender_is_closed: self.sender.is_closed(),
			})
		});

		match health_result {
			Ok(status) => status,
			Err(_) => {
				record_ws_error!("health_check_failed", "health_status");
				// Return degraded status
				HealthStatus {
					total_connections: self.connections.len(),
					metrics: self.get_metrics(),
					connection_states: ConnectionStateDistribution {
						active: 0,
						stale: 0,
						disconnected: 0,
					},
					sender_receiver_count: 0,
					sender_is_closed: true,
				}
			}
		}
	}

	async fn get_connection_state_distribution(&self) -> ConnectionStateDistribution {
		let mut active = 0;
		let mut stale = 0;
		let mut disconnected = 0;

		for entry in self.connections.iter() {
			match entry.value().state {
				ConnectionState::Active { .. } => active += 1,
				ConnectionState::Stale { .. } => stale += 1,
				ConnectionState::Disconnected { .. } => disconnected += 1,
			}
		}

		update_resource_usage!("active_connections", active as f64);
		update_resource_usage!("stale_connections", stale as f64);

		ConnectionStateDistribution { active, stale, disconnected }
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
	pub total_connections: usize,
	pub metrics: ConnectionMetricsSnapshot,
	pub connection_states: ConnectionStateDistribution,
	pub sender_receiver_count: usize,
	pub sender_is_closed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStateDistribution {
	pub active: usize,
	pub stale: usize,
	pub disconnected: usize,
}

async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<WebSocketFsm>) -> impl IntoResponse {
	ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: WebSocketFsm) {
	let (mut sender, mut receiver) = socket.split();

	// Create connection through FSM with proper resource management and instrumentation
	let (client_key, mut event_receiver) = match state.add_connection().await {
		Ok((key, rx)) => (key, rx),
		Err(e) => {
			record_ws_error!("connection_creation_failed", "websocket", e);
			error!("Failed to add connection: {}", e);
			return;
		}
	};

	record_system_event!("websocket_established", connection_id = client_key);
	info!("WebSocket connection established: {}", client_key);

	// Send initial ping with instrumentation
	let ping_event = Event::Ping;
	if let Ok(msg) = serde_json::to_string(&ping_event) {
		if let Err(e) = sender.send(Message::Text(msg)).await {
			record_ws_error!("initial_ping_failed", "websocket", e);
			error!("Failed to send initial ping to {}: {}", client_key, e);
		}
	}

	// Broadcast updated client count
	state.broadcast_client_count().await;

	// Forward events from broadcast channel to websocket with instrumentation
	let forward_task = {
		let client_key_clone = client_key.clone();
		tokio::spawn(async move {
			let mut message_count = 0u64;

			while let Ok(event) = event_receiver.recv().await {
				message_count += 1;

				let result = timed_ws_operation!("forward", "serialize", { serde_json::to_string(&event) });

				let msg = match result {
					Ok(json) => Message::Text(json),
					Err(e) => {
						record_ws_error!("serialization_failed", "forward", e);
						error!("Failed to serialize event for client {}: {}", client_key_clone, e);
						continue;
					}
				};

				let send_result = timed_ws_operation!("forward", "send", { sender.send(msg).await });

				if let Err(e) = send_result {
					record_ws_error!("forward_send_failed", "forward", e);
					error!("Failed to forward event to client {} (msg #{}): {}", client_key_clone, message_count, e);
					break;
				}

				// Log periodic forwarding stats
				if message_count % 100 == 0 {
					record_system_event!("forward_milestone", connection_id = client_key_clone, messages_forwarded = message_count);
					debug!("Forwarded {} messages to client {}", message_count, client_key_clone);
				}
			}

			record_system_event!("forward_ended", connection_id = client_key_clone, total_messages = message_count);
			debug!("Event forwarding ended for client {} after {} messages", client_key_clone, message_count);
		})
	};

	// Process incoming messages with enhanced error handling and instrumentation
	let mut message_count = 0u64;
	while let Some(result) = receiver.next().await {
		message_count += 1;

		match result {
			Ok(msg) => match msg {
				Message::Text(text) => {
					record_system_event!("message_received", connection_id = client_key, message_number = message_count, size_bytes = text.len());
					debug!("Received message #{} from {}: {} chars", message_count, client_key, text.len());

					let processing_result: Result<(), String> = timed_ws_operation!("websocket", "process_message", {
						state.process_message(&client_key, text).await;
						Ok(())
					});

					if processing_result.is_err() {
						record_ws_error!("message_processing_failed", "websocket");
					}
				}
				Message::Ping(_) => {
					record_system_event!("ping_received", connection_id = client_key);
					debug!("Received WebSocket ping from {}", client_key);
					if let Err(e) = state.update_client_ping(&client_key).await {
						record_ws_error!("ping_handling_failed", "websocket", e);
						warn!("Failed to update ping for {}: {}", client_key, e);
					}
				}
				Message::Pong(_) => {
					record_system_event!("pong_received", connection_id = client_key);
					debug!("Received WebSocket pong from {}", client_key);
					if let Err(e) = state.update_client_ping(&client_key).await {
						record_ws_error!("pong_handling_failed", "websocket", e);
						warn!("Failed to update pong for {}: {}", client_key, e);
					}
				}
				Message::Close(reason) => {
					record_system_event!("close_received", connection_id = client_key, reason = reason);
					info!("Client {} closed connection: {:?}", client_key, reason);
					break;
				}
				_ => {
					debug!("Ignored message type from {}", client_key);
				}
			},
			Err(e) => {
				record_ws_error!("websocket_error", "connection", e);
				error!("WebSocket error for {} (msg #{}): {}", client_key, message_count, e);
				break;
			}
		}
	}

	// Clean up through FSM with comprehensive logging and instrumentation
	record_system_event!("websocket_cleanup_started", connection_id = client_key, total_messages_processed = message_count);
	info!("Cleaning up connection {} after {} messages", client_key, message_count);

	let cleanup_result = health_check!("connection_cleanup", { state.remove_connection(&client_key, "Connection closed".to_string()).await });

	if let Err(e) = cleanup_result {
		record_ws_error!("cleanup_failed", "websocket", e);
		error!("Failed to remove connection {}: {}", client_key, e);
	}

	forward_task.abort();
	record_system_event!("websocket_cleanup_completed", connection_id = client_key);
	info!("Connection {} cleanup completed", client_key);
}

pub async fn init_websocket() -> WebSocketFsm {
	record_system_event!("websocket_init_started");
	let state = WebSocketFsm::new();

	// Start FSM processes with instrumentation
	state.start_timeout_monitor(Duration::from_secs(120));

	// Start system event monitoring for debugging with enhanced instrumentation
	let system_events = state.subscribe_to_system_events();
	tokio::spawn(async move {
		let mut events = system_events;
		record_system_event!("system_event_monitor_started");

		while let Ok(event) = events.recv().await {
			match event {
				SystemEvent::ConnectionStateChanged { connection_id, from, to } => {
					record_system_event!("connection_state_changed", connection_id = connection_id, from_state = from, to_state = to);
					info!("Connection {} state: {} -> {}", connection_id, from, to);
				}
				SystemEvent::MessageProcessed {
					message_id,
					connection_id,
					duration,
					result,
				} => {
					record_system_event!(
						"message_processed",
						message_id = message_id,
						connection_id = connection_id,
						duration_ms = duration.as_millis(),
						delivered = result.delivered,
						failed = result.failed
					);
					debug!(
						"Message {} from {} processed in {:?}: {} delivered, {} failed",
						message_id, connection_id, duration, result.delivered, result.failed
					);
				}
				SystemEvent::BroadcastFailed {
					event_type,
					error,
					affected_connections,
				} => {
					record_system_event!("broadcast_failed", event_type = event_type, error = error, affected_connections = affected_connections);
					error!("Broadcast failed for {:?} affecting {} connections: {}", event_type, affected_connections, error);
				}
				SystemEvent::ConnectionCleanup {
					connection_id,
					reason,
					resources_freed,
				} => {
					record_system_event!("connection_cleanup", connection_id = connection_id, reason = reason, resources_freed = resources_freed);
					info!("Connection {} cleaned up (reason: {}, resources freed: {})", connection_id, reason, resources_freed);
				}
			}
		}

		record_system_event!("system_event_monitor_ended");
	});

	record_system_event!("websocket_init_completed");
	info!("Enhanced FSM WebSocket system initialized with full observability and instrumentation");
	state
}

// Re-export for compatibility
pub use WebSocketFsm as WebSocketState;

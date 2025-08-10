use crate::*;
use async_broadcast::{broadcast, Receiver, Sender};
use axum::{
	extract::{
		ws::{Message, WebSocket, WebSocketUpgrade},
		ConnectInfo, State,
	},
	http::HeaderMap,
	response::IntoResponse,
	routing::get,
	Router,
};
use dashmap::DashMap;
use futures::{sink::SinkExt, stream::StreamExt};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
use tokio::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

pub mod broadcast;
pub mod connection;
pub mod message;
pub mod types;

pub(crate) use broadcast::spawn_event_forwarder;
pub use broadcast::BroadcastOutcome;
pub(crate) use connection::{cleanup_connection_with_stats, clear_connection, establish_connection, send_initial_handshake, ClientLimits};
pub use connection::{Connection, ConnectionId};
pub(crate) use message::process_incoming_messages;
pub use message::{EventMessage, MessageState, ProcessResult};
pub use types::*;

// Enhanced WebSocket FSM with comprehensive observability
#[derive(Clone)]
pub struct WebSocketFsm {
	connections: Arc<DashMap<String, Connection>>,
	sender: Sender<Event>,
	limits: ClientLimits,
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
			limits: ClientLimits::default(),
			metrics,
			system_events: system_sender,
		}
	}

	pub fn router(self) -> Router {
		Router::new().route("/ws", get(websocket_handler)).with_state(self)
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

async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<WebSocketFsm>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> impl IntoResponse {
	ws.on_upgrade(move |socket| handle_socket(socket, state, headers, addr))
}

// Orchestrates the WebSocket connection lifecycle
async fn handle_socket(socket: WebSocket, state: WebSocketFsm, headers: HeaderMap, addr: SocketAddr) {
	let (mut sender, receiver) = socket.split();

	// Establish connection through FSM
	let (conn_key, event_receiver) = match establish_connection(&state, &headers, &addr).await {
		Ok(connection) => connection,
		Err(_) => {
			record_ws_error!("connection refused", "handle_socket");
			return;
		} // Error already logged in establish_connection
	};

	// Send initial handshake
	if let Err(_) = send_initial_handshake(&mut sender, &conn_key).await {
		clear_connection(&state, &conn_key).await;
		return;
	}

	// Broadcast updated client count
	state.broadcast_client_count().await;

	// Start event forwarding task
	let forward_task = spawn_event_forwarder(sender, event_receiver, conn_key.clone());

	// Process incoming messages
	let message_count = process_incoming_messages(receiver, &state, &conn_key).await;

	// Clean up connection
	cleanup_connection_with_stats(&state, &conn_key, message_count, forward_task).await;
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

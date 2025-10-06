use crate::*;
use async_broadcast::Receiver;
use axum::{
	extract::{
		ws::{Message, WebSocketUpgrade},
		ConnectInfo, FromRef, State,
	},
	http::HeaderMap,
	response::IntoResponse,
	routing::get,
	Extension, Router,
};
use futures::{sink::SinkExt, stream::StreamExt};
use obs_websocket::{ObsConfig, ObsWebSocketManager, RetryConfig};
use serde::Serialize;
use some_transport::inmem::TransportLayer;
use std::{net::SocketAddr, sync::Arc};
use tokio::{sync::Notify, task::JoinHandle};
use tracing::{debug, error, info, warn};
use ws_connection::ConnectionStore;

pub mod broadcast;
pub mod connection;
pub mod heartbeat;
pub mod message;
pub mod middleware;
pub mod notify;
pub mod obs;
pub mod shutdown;
pub mod types;

use broadcast::spawn_event_forwarder;
pub use broadcast::BroadcastOutcome;
use connection::core::{cleanup_connection_with_stats, clear_connection, establish_connection, send_initial_handshake};
pub use heartbeat::{HeartbeatManager, HeartbeatPolicy};
use message::process_incoming_messages;
pub use message::{EventMessage, MessageState, ProcessResult};
use middleware::ConnectionGuard;
pub use middleware::{ConnectionLimitConfig, ConnectionLimiter};
pub use types::*;

// Enhanced WebSocket FSM with comprehensive observability
#[derive(Clone)]
pub struct WebSocketFsm {
	/// Domain layer: Connection actor handles
	store: Arc<ConnectionStore<EventType>>,

	/// Infrastructure layer: Transport/messaging
	transport: Arc<TransportLayer<Event>>,

	/// Heartbeat management
	heartbeat_manager: Arc<HeartbeatManager<EventType>>,

	/// Metrics
	metrics: Arc<ConnectionMetrics>,

	/// Subscriber notification
	subscriber_notify: Arc<Notify>,

	/// OBS integration
	pub obs_manager: Arc<ObsWebSocketManager>,

	/// Task handles
	heartbeat_task: Arc<std::sync::Mutex<Option<JoinHandle<()>>>>,
}

impl WebSocketFsm {
	/// Creates a new WebSocketFsm instance - only responsible for initialization
	pub fn new() -> Self {
		Self::with_policy(HeartbeatPolicy::default())
	}

	pub fn with_policy(policy: HeartbeatPolicy) -> Self {
		let (transport, _main_receiver) = TransportLayer::new(1000);
		let transport = Arc::new(transport);

		let store = Arc::new(ConnectionStore::<EventType>::new());
		let metrics = Arc::new(ConnectionMetrics::default());

		let heartbeat_manager = Arc::new(HeartbeatManager::new(store.clone(), transport.clone(), metrics.clone(), policy));

		let subscriber_notify = Arc::new(Notify::new());

		let obs_config = ObsConfig::default();
		let obs_manager = Arc::new(ObsWebSocketManager::new(obs_config, RetryConfig::default()));

		record_system_event!("fsm_initialized");
		update_resource_usage!("active_connections", 0.0);

		Self {
			store,
			transport,
			heartbeat_manager,
			metrics,
			subscriber_notify,
			obs_manager,
			heartbeat_task: Arc::new(std::sync::Mutex::new(None)),
		}
	}

	pub fn start(&self) {
		self.spawn_event_distribution_task();
		self.spawn_observability_monitor();
		let task = self.heartbeat_manager.clone().spawn();
		*self.heartbeat_task.lock().unwrap() = Some(task);
	}

	/// Subscribe to all events (both client and system)
	pub fn subscribe_to_events(&self) -> Receiver<Event> {
		self.transport.subscribe()
	}

	/// Subscribe to only system events (for observability tools)
	pub fn subscribe_to_system_events(&self) -> Receiver<Event> {
		self.transport.subscribe()
	}

	/// Subscribe to only client events (for testing/debugging)
	pub fn subscribe_to_client_events(&self) -> Receiver<Event> {
		self.transport.subscribe()
	}

	/// Emit a system event into the unified event stream
	async fn emit_system_event(&self, event: Event) {
		if event.is_system_event() {
			let _ = self.transport.broadcast(event).await;
		}
	}

	/// Emit a client event into the unified event stream
	async fn emit_client_event(&self, event: Event) {
		if event.is_client_event() {
			let _ = self.transport.broadcast(event).await;
		}
	}

	/// Spawn the observability monitor that filters and logs system events
	fn spawn_observability_monitor(&self) {
		let mut events = self.subscribe_to_system_events();

		tokio::spawn(async move {
			record_system_event!("observability_monitor_started");

			while let Ok(event) = events.recv().await {
				// Only process system events
				if !event.is_system_event() {
					continue;
				}

				match event {
					Event::ConnectionStateChanged { connection_id, from, to } => {
						record_system_event!("connection_state_changed", connection_id = connection_id, from_state = from, to_state = to);
						info!("Connection {} state: {} -> {}", connection_id, from, to);
					}
					Event::MessageProcessed {
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
					Event::BroadcastFailed {
						event_type,
						error,
						affected_connections,
					} => {
						record_system_event!("broadcast_failed", event_type = event_type, error = error, affected_connections = affected_connections);
						error!("Broadcast failed for {:?} affecting {} connections: {}", event_type, affected_connections, error);
					}
					Event::ConnectionCleanup {
						connection_id,
						reason,
						resources_freed,
					} => {
						record_system_event!("connection_cleanup", connection_id = connection_id, reason = reason, resources_freed = resources_freed);
						info!("Connection {} cleaned up (reason: {}, resources freed: {})", connection_id, reason, resources_freed);
					}
					_ => {}
				}
			}

			record_system_event!("observability_monitor_ended");
		});
	}

	pub async fn stop(&self) {
		self.heartbeat_manager.shutdown().await;
		if let Some(task) = self.heartbeat_task.lock().unwrap().take() {
			let _ = task.await;
		}
	}

	pub fn router<S>(self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
		AppState: FromRef<S>,
	{
		Router::new().route("/ws", get(websocket_handler))
	}

	async fn handle_subscription(&self, client_key: &str, event_types: Vec<EventType>, subscribe: bool) {
		if let Some(handle) = self.store.get(client_key) {
			let result = if subscribe {
				handle.subscribe(event_types.clone()).await
			} else {
				handle.unsubscribe(event_types.clone()).await
			};

			if let Err(e) = result {
				warn!("Failed to update subscriptions for {}: {}", client_key, e);
			} else {
				self.update_subscriber_state();
			}
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
		self.heartbeat_manager.record_ping(client_key).await;
		Ok(())
	}

	/// Send event to connection
	pub async fn send_event_to_connection(&self, client_key: &str, event: Event) -> Result<(), String> {
		if let Some(handle) = self.store.get(client_key) {
			let state = handle.get_state().await.map_err(|e| e.to_string())?;

			if !state.is_active {
				return Err(format!("Connection {} is not active", client_key));
			}

			self.transport.send_to_connection(client_key, event).await
		} else {
			Err(format!("Connection {} not found", client_key))
		}
	}

	async fn send_error_to_client(&self, client_key: &str, error: &str) {
		let error_event = Event::Error { message: error.to_string() };

		if let Err(e) = self.send_event_to_connection(client_key, error_event).await {
			record_ws_error!("error_send_failed", "connection", e);
			warn!("Failed to send error to client {}: {}", client_key, e);
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
			let stats = self.store.stats().await;

			check_invariant!(!self.transport.is_closed(), "transport_state", "Main transport channel is closed");

			check_invariant!(
				self.transport.receiver_count() > 0 || stats.total_connections == 0,
				"receiver_count",
				"No receivers but connections exist",
				expected: "receivers > 0 or connections == 0",
				actual: format!(
					"receivers: {}, connections: {}",
					self.transport.receiver_count(),
					stats.total_connections
				)
			);

			Ok(HealthStatus {
				total_connections: stats.total_connections,
				metrics,
				connection_states: ConnectionStateDistribution {
					active: stats.active_connections,
					stale: stats.stale_connections,
					disconnected: stats.disconnected_connections,
				},
				sender_receiver_count: self.transport.receiver_count(),
				sender_is_closed: self.transport.is_closed(),
				unique_clients: stats.unique_clients,
			})
		});

		match health_result {
			Ok(status) => status,
			Err(_) => {
				record_ws_error!("health_check_failed", "health_status");
				HealthStatus {
					total_connections: 0,
					metrics: self.get_metrics(),
					connection_states: ConnectionStateDistribution {
						active: 0,
						stale: 0,
						disconnected: 0,
					},
					sender_receiver_count: 0,
					sender_is_closed: true,
					unique_clients: 0,
				}
			}
		}
	}

	fn update_subscriber_state(&self) {
		self.subscriber_notify.notify_waiters();
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

async fn websocket_handler(
	ws: WebSocketUpgrade,
	State(state): State<AppState>,
	ConnectInfo(addr): ConnectInfo<SocketAddr>,
	headers: HeaderMap,
	Extension(connection_guard): Extension<ConnectionGuard>,
) -> impl IntoResponse {
	ws.on_upgrade(move |socket| handle_socket(socket, state.ws, headers, addr, connection_guard))
}

// Orchestrates the WebSocket connection lifecycle
async fn handle_socket(
	socket: axum::extract::ws::WebSocket,
	state: WebSocketFsm,
	headers: HeaderMap,
	addr: SocketAddr,
	_connection_guard: ConnectionGuard, // Keep this alive for the duration of the connection
) {
	let (mut sender, receiver) = socket.split();

	// Establish connection through FSM
	let (conn_key, event_receiver) = match establish_connection(&state, &headers, &addr).await {
		Ok(connection) => connection,
		Err(_) => {
			record_ws_error!("connection refused", "handle_socket");
			return;
		}
	};

	if let Err(_) = send_initial_handshake(&mut sender, &conn_key).await {
		clear_connection(&state, &conn_key).await;
		return;
	}

	state.broadcast_client_count().await;

	let forward_task = spawn_event_forwarder(sender, event_receiver, state.clone(), conn_key.clone());

	let message_count = process_incoming_messages(receiver, &state, &conn_key).await;

	cleanup_connection_with_stats(&state, &conn_key, message_count, forward_task).await;
}

pub async fn init_websocket() -> WebSocketFsm {
	init_websocket_with_policy(HeartbeatPolicy::default()).await
}

pub async fn init_websocket_with_policy(policy: HeartbeatPolicy) -> WebSocketFsm {
	record_system_event!("websocket_init_started");

	let state = WebSocketFsm::with_policy(policy);
	state.start();

	self.spawn_observability_monitor();

	info!("Actor-based WebSocket system initialized");
	state
}

// Re-export for compatibility
pub use WebSocketFsm as WebSocketState;

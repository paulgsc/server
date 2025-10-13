use crate::*;
use axum::{
	extract::{
		ws::{WebSocket, WebSocketUpgrade},
		ConnectInfo, FromRef, State,
	},
	http::{HeaderMap, StatusCode},
	response::IntoResponse,
	routing::get,
	Router,
};
use futures::{sink::SinkExt, stream::StreamExt};
use obs_websocket::{ObsConfig, ObsWebSocketManager, RetryConfig};
use serde::Serialize;
use some_transport::{InMemTransport, InMemTransportReceiver, Transport};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
	sync::Notify,
	task::JoinHandle,
	time::{timeout, Duration},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_connection::{ConnectionId, ConnectionStore};

pub mod broadcast;
pub mod connection;
pub mod heartbeat;
pub mod message;
pub mod notify;
pub mod obs;
pub mod shutdown;
pub mod types;

use broadcast::spawn_event_forwarder;
pub use broadcast::BroadcastResult;
use connection::core::{cleanup_connection_with_stats, clear_connection, establish_connection, send_initial_handshake};
pub use heartbeat::{HeartbeatManager, HeartbeatPolicy};
use message::process_incoming_messages;
pub use message::ProcessResult;
pub use types::*;

// Enhanced WebSocket FSM with comprehensive observability
#[derive(Clone)]
pub struct WebSocketFsm {
	/// Domain layer: Connection actor handles
	store: Arc<ConnectionStore<EventType>>,

	/// Infrastructure layer: Unified transport for ALL events
	transport: Arc<InMemTransport<Event>>,

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
	pub fn new(cancel_token: &CancellationToken) -> Self {
		Self::with_policy(HeartbeatPolicy::default(), cancel_token)
	}

	pub fn with_policy(policy: HeartbeatPolicy, cancel_token: &CancellationToken) -> Self {
		let transport = Arc::new(InMemTransport::new(1000));

		let store = Arc::new(ConnectionStore::<EventType>::new());
		let metrics = Arc::new(ConnectionMetrics::default());

		let clc = cancel_token.child_token();
		let heartbeat_manager = Arc::new(HeartbeatManager::new(store.clone(), transport.clone(), metrics.clone(), policy, &clc));

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

	pub fn start(&self, cancel_token: CancellationToken) {
		let event_clc = cancel_token.child_token().clone();
		let obs_clc = cancel_token.child_token().clone();

		self.spawn_event_distribution_task(event_clc);
		self.spawn_observability_monitor(obs_clc);
		let task = self.heartbeat_manager.clone().spawn();
		*self.heartbeat_task.lock().unwrap() = Some(task);
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

	/// Subscribe to all events (both client and system)
	pub fn subscribe_to_events(&self) -> InMemTransportReceiver<Event> {
		self.transport.subscribe()
	}

	/// Subscribe to only system events (for observability tools)
	/// Note: Returns all events - caller must filter for system events
	pub fn subscribe_to_system_events(&self) -> InMemTransportReceiver<Event> {
		self.transport.subscribe()
	}

	/// Subscribe to only client events (for testing/debugging)
	/// Note: Returns all events - caller must filter for client events
	pub fn subscribe_to_client_events(&self) -> InMemTransportReceiver<Event> {
		self.transport.subscribe()
	}

	/// Emit a system event into the unified event stream
	async fn emit_system_event(&self, event: Event) {
		if event.is_system_event() {
			let _ = self.transport.broadcast(event).await;
		}
	}

	/// Spawn the observability monitor that filters and logs system events
	fn spawn_observability_monitor(&self, cancel_token: CancellationToken) {
		let mut events = self.subscribe_to_system_events();

		tokio::spawn(async move {
			record_system_event!("observability_monitor_started");

			loop {
				tokio::select! {
					// Listen for shutdown signal
					_ = cancel_token.cancelled() => {
						info!("Observability monitor shutting down");
						break;
					}

					// Process events
					result = events.recv() => {
						match result {
							Ok(event) => {
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
							Err(e) => {
								error!("Observability monitor error: {}", e);
								break;
							}
						}
					}
				}
			}

			record_system_event!("observability_monitor_ended");
			info!("Observability monitor task exited");
		});
	}

	/// Handle subscription changes for a connection
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
				self.notify_subscription_change();
			}
		}
	}

	/// Update client ping timestamp via heartbeat manager
	async fn update_client_ping(&self, client_key: &str) -> Result<(), String> {
		self.heartbeat_manager.record_ping(client_key).await;
		Ok(())
	}

	/// Send event to a specific connection
	pub async fn send_event_to_connection(&self, client_key: &str, event: Event) -> Result<(), String> {
		if let Some(handle) = self.store.get(client_key) {
			let state = handle.get_state().await.map_err(|e| e.to_string())?;

			if !state.is_active {
				return Err(format!("Connection {} is not active", client_key));
			}

			self.transport.send(client_key, event).await.map_err(|e| e.to_string())
		} else {
			Err(format!("Connection {} not found", client_key))
		}
	}

	/// Send error message to a specific client
	async fn send_error_to_client(&self, client_key: &str, error: &str) {
		let error_event = Event::Error { message: error.to_string() };

		if let Err(e) = self.send_event_to_connection(client_key, error_event).await {
			record_ws_error!("error_send_failed", "connection", e);
			warn!("Failed to send error to client {}: {}", client_key, e);
		}
	}

	/// Get total number of active connections
	pub async fn get_client_count(&self) -> usize {
		self.store.len()
	}

	/// Get metrics snapshot
	pub fn get_metrics(&self) -> ConnectionMetricsSnapshot {
		self.metrics.get_snapshot()
	}

	/// Record a message processing event
	pub async fn record_message_processed(&self, message_id: MessageId, connection_id: ConnectionId, duration: Duration, result: ProcessResult) {
		self
			.emit_system_event(Event::MessageProcessed {
				message_id,
				connection_id,
				duration,
				result,
			})
			.await;
	}

	/// Record a broadcast failure
	pub async fn record_broadcast_failure(&self, event_type: EventType, error: String, affected_connections: usize) {
		self.metrics.broadcast_attempt(false);

		error!(
			event_type = ?event_type,
			error = %error,
			affected_connections = affected_connections,
			"Broadcast failed"
		);

		self
			.emit_system_event(Event::BroadcastFailed {
				event_type,
				error,
				affected_connections,
			})
			.await;
	}

	/// Record a successful broadcast
	pub async fn record_broadcast_success(&self, event_type: EventType, recipient_count: usize) {
		self.metrics.broadcast_attempt(true);

		info!(
			event_type = ?event_type,
			recipient_count = recipient_count,
			"Broadcast succeeded"
		);
	}

	/// Get health status for monitoring
	pub async fn get_health_status(&self) -> HealthStatus {
		let health_result: Result<HealthStatus, ()> = health_check!("health_status", {
			let metrics = self.get_metrics();
			let stats = self.store.stats().await;

			check_invariant!(!self.transport.is_closed(), "transport_state", "Main transport channel is closed");

			check_invariant!(
				self.transport.total_receivers() > 0 || stats.total_connections == 0,
				"receiver_count",
				"No receivers but connections exist",
				expected: "receivers > 0 or connections == 0",
				actual: format!(
					"receivers: {}, connections: {}",
					self.transport.total_receivers(),
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
				sender_receiver_count: self.transport.total_receivers(),
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
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
	pub total_connections: usize,
	pub metrics: ConnectionMetricsSnapshot,
	pub connection_states: ConnectionStateDistribution,
	pub sender_receiver_count: usize,
	pub sender_is_closed: bool,
	pub unique_clients: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStateDistribution {
	pub active: usize,
	pub stale: usize,
	pub disconnected: usize,
}

async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> impl IntoResponse {
	let client_id = addr.ip().to_string();
	let cancel_token = state.cancel_token.clone();
	info!("Incoming WS request from {client_id}");

	if !state.connection_guard.try_acquire_permit_hint() {
		warn!("Global limit exceeded — rejecting early");
		return (StatusCode::SERVICE_UNAVAILABLE, "Too many connections").into_response();
	}

	// Wrap acquire in a timeout (e.g., 5 seconds)
	match timeout(Duration::from_secs(5), state.connection_guard.acquire(client_id.clone())).await {
		Ok(Ok(permit)) => ws.on_upgrade(move |socket| handle_socket(socket, state.ws, headers, addr, permit, cancel_token)),
		Ok(Err(err)) => {
			use AcquireErrorKind::*;
			let reason = match err.kind {
				QueueFull => "Too many pending connections for this client",
				GlobalLimit => "Server is at capacity",
			};
			error!("Rejecting WS for {client_id}: {reason}");
			(StatusCode::SERVICE_UNAVAILABLE, reason).into_response()
		}
		Err(_timeout_elapsed) => {
			error!("Timeout waiting for permit for {client_id}");
			(StatusCode::REQUEST_TIMEOUT, "Connection acquisition timed out").into_response()
		}
	}
}

/// Orchestrates the WebSocket connection lifecycle
async fn handle_socket(socket: WebSocket, state: WebSocketFsm, headers: HeaderMap, addr: SocketAddr, permit: ConnectionPermit, cancel_token: CancellationToken) {
	let (mut sender, receiver) = socket.split();

	// Establish connection through FSM
	let (conn_key, event_receiver) = match establish_connection(&state, &headers, &addr, &cancel_token).await {
		Ok(connection) => connection,
		Err(_) => {
			record_ws_error!("connection_refused", "handle_socket");
			return;
		}
	};

	if send_initial_handshake(&mut sender).await.is_err() {
		clear_connection(&state, &conn_key).await;
		return;
	}

	state.broadcast_client_count().await;

	// Pass cancel token to both tasks
	let forward_cancel = cancel_token.child_token().clone();
	let process_cancel = cancel_token.child_token().clone();

	let forward_task = spawn_event_forwarder(sender, event_receiver, state.clone(), conn_key.clone(), forward_cancel);

	let message_count = process_incoming_messages(receiver, &state, &conn_key, process_cancel).await;

	cleanup_connection_with_stats(&state, &conn_key, message_count, forward_task).await;
	permit.release();
}

/// Initialize WebSocket system with default policy
pub async fn init_websocket(cancel_token: CancellationToken) -> WebSocketFsm {
	init_websocket_with_policy(HeartbeatPolicy::default(), cancel_token).await
}

/// Initialize WebSocket system with custom heartbeat policy
pub async fn init_websocket_with_policy(policy: HeartbeatPolicy, cancel_token: CancellationToken) -> WebSocketFsm {
	record_system_event!("websocket_init_started");

	let state = WebSocketFsm::with_policy(policy, &cancel_token);
	state.start(cancel_token.clone());

	record_system_event!("websocket_init_completed");
	info!("Actor-based WebSocket system initialized");

	state
}

// Re-export for compatibility
pub use WebSocketFsm as WebSocketState;

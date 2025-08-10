use super::*;
use crate::utils::generate_uuid;
use async_broadcast::{broadcast, Receiver, Sender};
use axum::extract::ws::{Message, WebSocket};
use futures::stream::SplitSink;
use std::{collections::HashSet, fmt};
use tokio::time::{Duration, Instant};
use tracing::info;
// Connection ID type for type safety
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionId([u8; 32]);

impl ConnectionId {
	pub fn new() -> Self {
		Self(generate_uuid())
	}

	pub fn from_buffer(buffer: [u8; 32]) -> Self {
		Self(buffer)
	}

	pub fn as_string(&self) -> String {
		// Convert to hex string for reliable string representation
		hex::encode(&self.0)
	}

	pub fn as_bytes(&self) -> &[u8; 32] {
		&self.0
	}
}

impl fmt::Display for ConnectionId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.as_string())
	}
}

#[derive(Debug)]
pub struct Connection {
	pub id: ConnectionId,
	pub established_at: Instant,
	pub state: ConnectionState,
	pub sender: Sender<Event>,
	pub subscriptions: HashSet<EventType>,
	pub message_count: u64,
	pub last_message_at: Instant,
}

impl Connection {
	pub fn new() -> (Self, Receiver<Event>) {
		let (mut sender, receiver) = broadcast::<Event>(1);
		sender.set_await_active(false);
		sender.set_overflow(true);

		let mut subscriptions = HashSet::new();
		subscriptions.insert(EventType::Ping);
		subscriptions.insert(EventType::Pong);
		subscriptions.insert(EventType::Error);
		subscriptions.insert(EventType::ClientCount);

		let connection = Self {
			id: ConnectionId::new(),
			established_at: Instant::now(),
			state: ConnectionState::Active { last_ping: Instant::now() },
			sender,
			subscriptions,
			message_count: 0,
			last_message_at: Instant::now(),
		};

		(connection, receiver)
	}

	pub fn update_ping(&mut self) -> Result<ConnectionState, String> {
		let now = Instant::now();
		let old_state = self.state.clone();
		match &mut self.state {
			ConnectionState::Active { last_ping } => {
				*last_ping = now;
				self.last_message_at = now;
				Ok(old_state)
			}
			_ => Err("Cannot update ping on non-active connection".to_string()),
		}
	}

	pub fn subscribe(&mut self, event_types: Vec<EventType>) -> usize {
		let initial_count = self.subscriptions.len();
		for t in event_types {
			self.subscriptions.insert(t);
		}
		self.subscriptions.len() - initial_count
	}

	pub fn unsubscribe(&mut self, event_types: Vec<EventType>) -> usize {
		let initial_count = self.subscriptions.len();
		for t in event_types {
			self.subscriptions.remove(&t);
		}
		initial_count - self.subscriptions.len()
	}

	pub fn mark_stale(&mut self, reason: String) -> Result<ConnectionState, String> {
		let old_state = self.state.clone();
		match &self.state {
			ConnectionState::Active { last_ping } => {
				self.state = ConnectionState::Stale { last_ping: *last_ping, reason };
				Ok(old_state)
			}
			_ => Err("Can only mark active connections as stale".to_string()),
		}
	}

	pub fn disconnect(&mut self, reason: String) -> Result<ConnectionState, String> {
		let old_state = self.state.clone();
		self.state = ConnectionState::Disconnected {
			reason,
			disconnected_at: Instant::now(),
		};
		Ok(old_state)
	}

	pub fn is_active(&self) -> bool {
		matches!(self.state, ConnectionState::Active { .. })
	}

	pub fn is_stale(&self, timeout: Duration) -> bool {
		match &self.state {
			ConnectionState::Active { last_ping } => Instant::now().duration_since(*last_ping) > timeout,
			ConnectionState::Stale { .. } => true,
			_ => false,
		}
	}

	pub fn is_subscribed_to(&self, event_type: &EventType) -> bool {
		self.subscriptions.contains(event_type)
	}

	pub async fn send_event(&self, event: Event) -> Result<(), String> {
		if !self.is_active() {
			return Err(format!("Cannot send to non-active connection (state: {})", self.state));
		}

		match self.sender.broadcast(event).await {
			Ok(_) => Ok(()),
			Err(e) => Err(format!("Failed to send event to client channel: {}", e)),
		}
	}

	pub fn increment_message_count(&mut self) {
		self.message_count += 1;
		self.last_message_at = Instant::now();
	}
}

impl WebSocketFsm {
	/// Adds a connection, removing any stale or duplicate from same key first
	pub async fn add_connection(&self) -> Result<(String, Receiver<Event>), String> {
		let (connection, receiver) = Connection::new();
		let client_key = connection.id.as_string();
		let connection_id = connection.id.clone();

		// --- Proactive duplicate cleanup ---
		if let Some((_, mut old_conn)) = self.connections.remove(&client_key) {
			let old_id = old_conn.id.clone();
			let duration = old_conn.established_at.elapsed();
			let _ = old_conn.disconnect("Replaced by new connection".into());

			cleanup_connection!(old_id, "Replaced by new connection", duration, true);
			self.metrics.connection_removed(old_conn.is_active());
		}

		self.connections.insert(client_key.clone(), connection);
		record_connection_event!("created", connection_id);
		update_connection_state!("", "active");
		update_resource_usage!("active_connections", self.connections.len() as f64);

		self.metrics.connection_created();

		info!("Connection {} added successfully", connection_id);
		Ok((client_key, receiver))
	}

	pub async fn remove_connection(&self, client_key: &str, reason: String) -> Result<(), String> {
		if let Some((_, mut connection)) = self.connections.remove(client_key) {
			let connection_id = connection.id.clone();
			let was_active = connection.is_active();
			let duration = connection.established_at.elapsed();

			let old_state = connection.disconnect(reason.clone())?;
			self.metrics.connection_removed(was_active);

			cleanup_connection!(connection_id, &reason, duration, true);
			update_resource_usage!("active_connections", self.connections.len() as f64);

			record_system_event!(
				"connection_state_changed",
				connection_id = connection_id,
				from_state = old_state,
				to_state = connection.state
			);

			info!("Connection {} removed: {}", connection_id, reason);
			self.broadcast_client_count().await;
		}
		Ok(())
	}

	/// Optimized timeout monitor with batch cleanup and zero-alloc scanning
	pub fn start_timeout_monitor(&self, timeout: Duration) {
		let connections = self.connections.clone();
		let metrics = self.metrics.clone();
		let sender = self.sender.clone();

		let mut keys_to_remove: Vec<String> = Vec::with_capacity(64);
		let interval_duration = Duration::from_secs(10); // faster than 30s for responsiveness

		tokio::spawn(async move {
			let mut interval = tokio::time::interval(interval_duration);

			loop {
				interval.tick().await;

				// Health check
				let health_result: Result<(), String> = health_check!("timeout_monitor", {
					let total_connections = connections.len();
					let metrics_snapshot = metrics.get_snapshot();
					let expected_active = metrics_snapshot.total_created - metrics_snapshot.total_removed;

					check_invariant!(
							total_connections as u64 == expected_active,
							"connection_count",
							"Connection count mismatch",
							expected: expected_active,
							actual: total_connections as u64
					);

					log_health_snapshot!(metrics, total_connections);
					Ok(())
				});
				if health_result.is_err() {
					record_ws_error!("health_check_failed", "timeout_monitor");
				}

				// Collect stale keys without allocating each loop
				keys_to_remove.clear();
				for entry in connections.iter() {
					if entry.value().is_stale(timeout) {
						keys_to_remove.push(entry.key().clone());
					}
				}

				// Batch cleanup
				const BATCH_SIZE: usize = 64;
				let mut cleaned_up = 0usize;

				for chunk in keys_to_remove.chunks(BATCH_SIZE) {
					for client_key in chunk {
						if let Some((_, mut conn)) = connections.remove(client_key) {
							let connection_id = conn.id.clone();
							let duration = conn.established_at.elapsed();
							let _ = conn.disconnect("Timeout cleanup".into());

							cleanup_connection!(connection_id, "Timeout cleanup", duration, true);
							record_system_event!("connection_state_changed", connection_id = connection_id, from_state = conn.state, to_state = conn.state);
							metrics.connection_marked_stale();
							cleaned_up += 1;
						}
					}
					tokio::task::yield_now().await;
				}

				if cleaned_up > 0 {
					record_system_event!("cleanup_completed", connections_cleaned = cleaned_up);
					info!("Cleaned up {} stale connections", cleaned_up);
					let count = connections.len();
					let _ = sender.broadcast(Event::ClientCount { count }).await;
					update_resource_usage!("active_connections", count as f64);
				}
			}
		});
	}
}

pub(crate) async fn establish_connection(state: &WebSocketFsm) -> Result<(String, Receiver<Event>), ()> {
	match state.add_connection().await {
		Ok((key, rx)) => {
			record_system_event!("websocket_established", connection_id = key);
			info!("WebSocket connection established: {}", key);
			Ok((key, rx))
		}
		Err(e) => {
			record_ws_error!("connection_creation_failed", "websocket", e);
			error!("Failed to add connection: {}", e);
			Err(())
		}
	}
}

// Sends initial handshake (ping) to client
pub(crate) async fn send_initial_handshake(sender: &mut SplitSink<WebSocket, Message>, client_key: &str) -> Result<(), ()> {
	let ping_event = Event::Ping;
	if let Ok(msg) = serde_json::to_string(&ping_event) {
		if let Err(e) = sender.send(Message::Text(msg)).await {
			record_ws_error!("initial_ping_failed", "websocket", e);
			error!("Failed to send initial ping to {}: {}", client_key, e);
			return Err(());
		}
	}
	Ok(())
}

// Basic connection cleanup
pub(crate) async fn clear_connection(state: &WebSocketFsm, client_key: &str) {
	let cleanup_result = health_check!("connection_cleanup", {
		state.remove_connection(client_key, "Connection failed during setup".to_string()).await
	});

	if let Err(e) = cleanup_result {
		record_ws_error!("cleanup_failed", "websocket", e);
		error!("Failed to remove connection {}: {}", client_key, e);
	}
}

// Comprehensive connection cleanup with statistics
pub(crate) async fn cleanup_connection_with_stats(state: &WebSocketFsm, client_key: &str, message_count: u64, forward_task: tokio::task::JoinHandle<()>) {
	record_system_event!("websocket_cleanup_started", connection_id = client_key, total_messages_processed = message_count);
	info!("Cleaning up connection {} after {} messages", client_key, message_count);

	let cleanup_result = health_check!("connection_cleanup", { state.remove_connection(client_key, "Connection closed".to_string()).await });

	if let Err(e) = cleanup_result {
		record_ws_error!("cleanup_failed", "websocket", e);
		error!("Failed to remove connection {}: {}", client_key, e);
	}

	forward_task.abort();
	record_system_event!("websocket_cleanup_completed", connection_id = client_key);
	info!("Connection {} cleanup completed", client_key);
}

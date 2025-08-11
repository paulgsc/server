use super::*;
use crate::utils::generate_uuid;
use async_broadcast::{broadcast, Receiver, Sender};
use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use futures::stream::SplitSink;
use std::{collections::HashSet, fmt, net::SocketAddr};
use tokio::time::Duration;
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientId(String);

impl ClientId {
	pub fn from_request(headers: &HeaderMap, addr: &SocketAddr) -> Self {
		// Priority order for client identification:
		// 1. X-Client-ID (for authenticated clients)
		// 2. X-Forwarded-For + User-Agent hash (for proxied clients)
		// 3. IP + User-Agent hash (for direct clients)
		// 4. IP only (fallback)

		if let Some(client_id) = headers.get("x-client-id").and_then(|v| v.to_str().ok()) {
			if !client_id.is_empty() && client_id.len() <= 64 {
				return Self(format!("auth:{}", client_id));
			}
		}

		let user_agent = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("unknown");

		let user_agent_hash = {
			use std::hash::{Hash, Hasher};
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			user_agent.hash(&mut hasher);
			hasher.finish()
		};

		// Check for forwarded IP (behind proxy/load balancer)
		if let Some(forwarded_for) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
			if let Some(client_ip) = forwarded_for.split(',').next().map(|s| s.trim()) {
				return Self(format!("proxy:{}:{:x}", client_ip, user_agent_hash));
			}
		}

		// Real connecting IP
		Self(format!("direct:{}:{:x}", addr.ip(), user_agent_hash))
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl fmt::Display for ClientId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug)]
pub struct Connection {
	pub id: ConnectionId,
	pub client_id: ClientId,
	pub established_at: Instant,
	pub state: ConnectionState,
	pub sender: Sender<Event>,
	pub subscriptions: HashSet<EventType>,
	pub message_count: u64,
	pub last_message_at: Instant,
	pub source_addr: SocketAddr,
}

impl Connection {
	pub fn new(client_id: ClientId, source_addr: SocketAddr) -> (Self, Receiver<Event>) {
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
			client_id,
			established_at: Instant::now(),
			state: ConnectionState::Active { last_ping: Instant::now() },
			sender,
			subscriptions,
			message_count: 0,
			last_message_at: Instant::now(),
			source_addr,
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
				update_connection_state!(old_state.as_str(), "disconnected");
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
		update_connection_state!(old_state.as_str(), "disconnected");
		Ok(old_state)
	}

	pub fn is_active(&self) -> bool {
		matches!(self.state, ConnectionState::Active { .. })
	}

	// Helper method to check if connection should be marked as stale
	pub fn should_be_stale(&self, timeout: Duration) -> bool {
		match &self.state {
			ConnectionState::Active { last_ping } => Instant::now().duration_since(*last_ping) > timeout,
			_ => false,
		}
	}

	// Check if connection is already in stale state
	pub fn is_stale(&self) -> bool {
		matches!(self.state, ConnectionState::Stale { .. })
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

#[derive(Debug, Clone)]
pub struct ClientLimits {
	pub max_connections_per_client: usize,
	pub max_total_connections: usize,
	pub enforce_limits: bool,
}

impl Default for ClientLimits {
	fn default() -> Self {
		Self {
			max_connections_per_client: 3,
			max_total_connections: 50,
			enforce_limits: true,
		}
	}
}

impl WebSocketFsm {
	/// Adds a connection, removing any stale or duplicate from same key first
	pub async fn add_connection(&self, headers: &HeaderMap, addr: &SocketAddr) -> Result<(String, Receiver<Event>), String> {
		let client_id = ClientId::from_request(headers, addr);

		let (connection, receiver) = Connection::new(client_id.clone(), *addr);
		let client_key = connection.id.as_string();
		let connection_id = connection.id.clone();

		self.connections.insert(client_key.clone(), connection);

		record_connection_event!("created", connection_id);
		update_connection_state!("", "active");
		update_resource_usage!("active_connections", self.connections.len() as f64);

		self.metrics.connection_created();

		info!("Connection {} added successfully", connection_id);
		Ok((client_key, receiver))
	}

	// Get connections by client ID
	pub async fn get_client_connections(&self, client_id: &ClientId) -> Vec<String> {
		self
			.connections
			.iter()
			.filter(|entry| entry.value().client_id == *client_id)
			.map(|entry| entry.key().clone())
			.collect()
	}

	pub async fn remove_connection(&self, client_key: &str, reason: String) -> Result<(), String> {
		if let Some((_, mut connection)) = self.connections.remove(client_key) {
			let connection_id = connection.id.clone();
			let client_id = connection.client_id.clone();
			let was_active = connection.is_active();
			let duration = connection.established_at.elapsed();

			let old_state = connection.disconnect(reason.clone())?;
			self.metrics.connection_removed(was_active);

			update_connection_state!(old_state.as_str(), "disconnected");

			cleanup_connection!(connection_id, &reason, duration, true);
			update_resource_usage!("active_connections", self.connections.len() as f64);

			record_system_event!(
				"connection_state_changed",
				connection_id = connection_id,
				client_id = client_id,
				from_state = old_state,
				to_state = connection.state
			);

			info!("Connection {} removed: {}", connection_id, reason);
			self.broadcast_client_count().await;
		}
		Ok(())
	}

	/// Optimized timeout monitor with proper state transitions
	pub fn start_timeout_monitor(&self, timeout: Duration) {
		let connections = self.connections.clone();
		let metrics = self.metrics.clone();
		let sender = self.sender.clone();

		let mut keys_to_mark_stale: Vec<String> = Vec::with_capacity(64);
		let mut keys_to_remove: Vec<String> = Vec::with_capacity(64);
		let interval_duration = Duration::from_secs(10);

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

					let mut client_counts: std::collections::HashMap<ClientId, usize> = std::collections::HashMap::new();
					for entry in connections.iter() {
						if entry.value().is_active() {
							*client_counts.entry(entry.value().client_id.clone()).or_insert(0) += 1;
						}
					}

					for (client_id, count) in client_counts.iter() {
						if *count > 10 {
							// Warning threshold
							record_system_event!("client_high_connection_count", client_id = client_id, count = count);
						}
					}

					log_health_snapshot!(metrics, total_connections);
					Ok(())
				});

				if health_result.is_err() {
					record_ws_error!("health_check_failed", "timeout_monitor");
				}

				// Step 1: Mark active connections as stale if they've timed out
				keys_to_mark_stale.clear();
				for entry in connections.iter() {
					let connection = entry.value();
					if connection.should_be_stale(timeout) {
						keys_to_mark_stale.push(entry.key().clone());
					}
				}

				// Mark connections as stale
				let mut newly_stale = 0usize;
				for client_key in &keys_to_mark_stale {
					if let Some(mut entry) = connections.get_mut(client_key) {
						let connection = entry.value_mut();
						if connection.is_active() {
							let connection_id = connection.id.clone();
							let client_id = connection.client_id.clone();

							if let Ok(old_state) = connection.mark_stale("Connection timeout".to_string()) {
								record_system_event!(
									"connection_state_changed",
									connection_id = connection_id,
									client_id = client_id,
									from_state = old_state,
									to_state = connection.state
								);

								metrics.connection_marked_stale();

								update_resource_usage!("active_connections", connections.iter().filter(|e| e.value().is_active()).count() as f64);
								update_resource_usage!("stale_connections", connections.iter().filter(|e| e.value().is_stale()).count() as f64);

								newly_stale += 1;
							}
						}
					}
				}

				if newly_stale > 0 {
					info!("Marked {} connections as stale", newly_stale);
				}

				// Step 2: Collect connections that are in stale state for removal
				keys_to_remove.clear();
				for entry in connections.iter() {
					if entry.value().is_stale() {
						keys_to_remove.push(entry.key().clone());
					}
				}

				// Step 3: Remove stale connections in batches
				const BATCH_SIZE: usize = 64;
				let mut cleaned_up = 0usize;

				for chunk in keys_to_remove.chunks(BATCH_SIZE) {
					for client_key in chunk {
						if let Some((_, mut conn)) = connections.remove(client_key) {
							let connection_id = conn.id.clone();
							let client_id = conn.client_id.clone();
							let duration = conn.established_at.elapsed();
							let old_state = conn.state.clone();

							let _ = conn.disconnect("Stale connection cleanup".into());

							cleanup_connection!(connection_id, "Stale connection cleanup", duration, true);
							record_system_event!(
								"connection_state_changed",
								connection_id = connection_id,
								client_id = client_id,
								from_state = old_state,
								to_state = conn.state
							);
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

pub(crate) async fn establish_connection(state: &WebSocketFsm, headers: &HeaderMap, addr: &SocketAddr) -> Result<(String, Receiver<Event>), ()> {
	match state.add_connection(headers, addr).await {
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
pub(crate) async fn send_initial_handshake(sender: &mut SplitSink<WebSocket, Message>, conn_key: &str) -> Result<(), ()> {
	let ping_event = Event::Ping;
	if let Ok(msg) = serde_json::to_string(&ping_event) {
		if let Err(e) = sender.send(Message::Text(msg)).await {
			record_ws_error!("initial_ping_failed", "websocket", e);
			error!("Failed to send initial ping to {}: {}", conn_key, e);
			return Err(());
		}
	}
	Ok(())
}

// Basic connection cleanup
pub(crate) async fn clear_connection(state: &WebSocketFsm, conn_key: &str) {
	let cleanup_result = health_check!("connection_cleanup", {
		state.remove_connection(conn_key, "Connection failed during setup".to_string()).await
	});

	if let Err(e) = cleanup_result {
		record_ws_error!("cleanup_failed", "websocket", e);
		error!("Failed to remove connection {}: {}", conn_key, e);
	}
}

// Comprehensive connection cleanup with statistics
pub(crate) async fn cleanup_connection_with_stats(state: &WebSocketFsm, conn_key: &str, message_count: u64, forward_task: tokio::task::JoinHandle<()>) {
	record_system_event!("websocket_cleanup_started", connection_id = conn_key, total_messages_processed = message_count);
	info!("Cleaning up connection {} after {} messages", conn_key, message_count);

	let cleanup_result = health_check!("connection_cleanup", { state.remove_connection(conn_key, "Connection closed".to_string()).await });

	if let Err(e) = cleanup_result {
		record_ws_error!("cleanup_failed", "websocket", e);
		error!("Failed to remove connection {}: {}", conn_key, e);
	}

	forward_task.abort();
	record_system_event!("websocket_cleanup_completed", connection_id = conn_key);
	info!("Connection {} cleanup completed", conn_key);
}

use crate::utils::generate_uuid;
use crate::websocket::*;
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
			client_id: client_id.clone(),
			established_at: Instant::now(),
			state: ConnectionState::Active { last_ping: Instant::now() },
			sender,
			subscriptions,
			message_count: 0,
			last_message_at: Instant::now(),
			source_addr,
		};

		record_connection_created!(connection.id, client_id);

		(connection, receiver)
	}

	pub fn update_ping(&mut self) -> Result<ConnectionState, String> {
		let now = Instant::now();
		let old_state = self.state.clone();
		match &mut self.state {
			ConnectionState::Active { last_ping } => {
				*last_ping = now;
				self.last_message_at = now;
				record_connection_message!(self.id, "ping");
				Ok(old_state)
			}
			_ => {
				let error = "Cannot update ping on non-active connection".to_string();
				record_connection_error!("ping_update_failed", "operation", self.id, error.clone());
				Err(error)
			}
		}
	}

	pub fn subscribe(&mut self, event_types: Vec<EventType>) -> usize {
		let initial_count = self.subscriptions.len();
		for t in event_types.iter() {
			self.subscriptions.insert(t.clone());
		}
		let delta = self.subscriptions.len() - initial_count;

		if delta > 0 {
			record_subscription_change!(self.id, "subscribe", &event_types, delta);
		}

		delta
	}

	pub fn unsubscribe(&mut self, event_types: Vec<EventType>) -> usize {
		let initial_count = self.subscriptions.len();
		for t in event_types.iter() {
			self.subscriptions.remove(t);
		}
		let delta = initial_count - self.subscriptions.len();

		if delta > 0 {
			record_subscription_change!(self.id, "unsubscribe", &event_types, delta);
		}

		delta
	}

	pub fn mark_stale(&mut self, reason: String) -> Result<ConnectionState, String> {
		let old_state = self.state.clone();
		match &self.state {
			ConnectionState::Active { last_ping } => {
				let new_state = ConnectionState::Stale { last_ping: *last_ping, reason };
				record_connection_state_change!(self.id, self.client_id, old_state, new_state);
				self.state = new_state;
				Ok(old_state)
			}
			_ => {
				let error = "Can only mark active connections as stale".to_string();
				record_connection_error!("mark_stale_failed", "operation", self.id, error.clone());
				Err(error)
			}
		}
	}

	pub fn disconnect(&mut self, reason: String) -> Result<ConnectionState, String> {
		let old_state = self.state.clone();
		let new_state = ConnectionState::Disconnected {
			reason,
			disconnected_at: Instant::now(),
		};
		record_connection_state_change!(self.id, self.client_id, old_state, new_state);
		self.state = new_state;
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
			let error = format!("Cannot send to non-active connection (state: {})", self.state);
			record_connection_error!("send_failed", "operation", self.id, error.clone());
			return Err(error);
		}

		match self.sender.broadcast(event).await {
			Ok(_) => {
				record_connection_message!(self.id, "event_sent");
				Ok(())
			}
			Err(e) => {
				let error = format!("Failed to send event to client channel: {}", e);
				record_connection_error!("send_failed", "operation", self.id, error.clone());
				Err(error)
			}
		}
	}

	pub fn increment_message_count(&mut self) {
		self.message_count += 1;
		self.last_message_at = Instant::now();
		record_connection_message!(self.id, "message_processed");
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

			let _ = connection.disconnect(reason.clone())?;
			self.metrics.connection_removed(was_active);

			record_connection_removed!(connection_id, client_id, duration, reason);

			info!("Connection {} removed: {}", connection_id, reason);
			self.broadcast_client_count().await;
		}
		Ok(())
	}

	/// Optimized timeout monitor with proper state transitions
	pub fn start_timeout_monitor(&self, timeout: Duration, shutdown_token: CancellationToken) {
		let connections = self.connections.clone();
		let metrics = self.metrics.clone();
		let sender = self.sender.clone();

		let mut keys_to_mark_stale = Vec::with_capacity(64);
		let mut keys_to_remove = Vec::with_capacity(64);

		let interval_duration = Duration::from_secs(10);

		tokio::spawn(async move {
			let mut interval = tokio::time::interval(interval_duration);

			loop {
				tokio::select! {
					_ = shutdown_token.cancelled() => {
						tracing::info!("Timeout monitor received shutdown signal");
						break;
					}
					_ = interval.tick() => {
						if let Err(e) = Self::process_timeout_cycle(
							&connections,
							&metrics,
							&sender,
							timeout,
							&mut keys_to_mark_stale,
							&mut keys_to_remove,
						).await {
							tracing::error!("Error in timeout monitor cycle: {}", e);
						}
						tokio::task::yield_now().await;
					}
				}
			}
			tracing::info!("Timeout monitor shutting down gracefully");
		});
	}

	async fn process_timeout_cycle(
		connections: &DashMap<String, Connection>,
		metrics: &ConnectionMetrics,
		sender: &async_broadcast::Sender<Event>,
		timeout: Duration,
		keys_to_mark_stale: &mut Vec<String>,
		keys_to_remove: &mut Vec<String>,
	) -> Result<(), String> {
		Self::check_health(connections, metrics).await;

		let newly_stale = Self::mark_stale_connections(connections, metrics, timeout, keys_to_mark_stale).await;

		if newly_stale > 0 {
			tracing::info!("Marked {} connections as stale", newly_stale);
		}

		let cleaned_up = Self::cleanup_stale_connections(connections, sender, keys_to_remove).await?;

		if cleaned_up > 0 {
			tracing::info!("Cleaned up {} stale connections", cleaned_up);
		}

		Ok(())
	}

	async fn check_health(connections: &DashMap<String, Connection>, metrics: &ConnectionMetrics) {
		let result: Result<(), String> = health_check!("timeout_monitor", {
			let total_connections = connections.len();
			let snapshot = metrics.get_snapshot();
			let expected_active = snapshot.total_created - snapshot.total_removed;

			check_invariant!(
				total_connections as u64 == expected_active,
				"connection_count",
				"Mismatch in connection count",
				expected: expected_active,
				actual: total_connections as u64
			);

			let mut client_counts = std::collections::HashMap::new();
			for entry in connections.iter() {
				if entry.value().is_active() {
					*client_counts.entry(entry.value().client_id.clone()).or_insert(0) += 1;
				}
				if client_counts.len() % 100 == 0 {
					tokio::task::yield_now().await;
				}
			}

			for (client_id, count) in client_counts {
				if count > 10 {
					record_system_event!("client_high_connection_count", client_id = client_id, count = count);
				}
			}

			log_health_snapshot!(metrics, total_connections);
			Ok(())
		});

		if result.is_err() {
			record_ws_error!("health_check_failed", "timeout_monitor");
		}
	}

	async fn mark_stale_connections(connections: &DashMap<String, Connection>, metrics: &ConnectionMetrics, timeout: Duration, keys_to_mark_stale: &mut Vec<String>) -> usize {
		keys_to_mark_stale.clear();
		for entry in connections.iter() {
			if entry.value().should_be_stale(timeout) {
				keys_to_mark_stale.push(entry.key().clone());
			}
		}

		let mut newly_stale = 0;
		for (idx, client_key) in keys_to_mark_stale.iter().enumerate() {
			if let Some(mut entry) = connections.get_mut(client_key) {
				if entry.value().is_active() {
					if let Ok(_) = entry.value_mut().mark_stale("Connection timeout".into()) {
						metrics.connection_marked_stale();
						newly_stale += 1;
					}
				}
			}
			if idx % 10 == 0 {
				tokio::task::yield_now().await;
			}
		}
		newly_stale
	}

	async fn cleanup_stale_connections(
		connections: &DashMap<String, Connection>,
		sender: &async_broadcast::Sender<Event>,
		keys_to_remove: &mut Vec<String>,
	) -> Result<usize, String> {
		keys_to_remove.clear();
		for entry in connections.iter() {
			if entry.value().is_stale() {
				keys_to_remove.push(entry.key().clone());
			}
		}

		let mut cleaned_up = 0;
		for chunk in keys_to_remove.chunks(64) {
			for key in chunk {
				if let Some((_, mut conn)) = connections.remove(key) {
					let _ = conn.disconnect("Stale connection cleanup".into());
					cleaned_up += 1;
				}
			}
			tokio::task::yield_now().await;
		}

		if cleaned_up > 0 {
			let count = connections.len();
			let _ = sender.broadcast(Event::ClientCount { count }).await;
			update_resource_usage!("active_connections", count as f64);
		}

		Ok(cleaned_up)
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
			record_connection_error!("creation_failed", "creation", e);
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
			record_connection_error!("handshake_failed", "creation", e);
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
		record_connection_error!("cleanup_failed", "cleanup", e);
		error!("Failed to remove connection {}: {}", conn_key, e);
	}
}

// Comprehensive connection cleanup with statistics
pub(crate) async fn cleanup_connection_with_stats(state: &WebSocketFsm, conn_key: &str, message_count: u64, forward_task: tokio::task::JoinHandle<()>) {
	record_system_event!("websocket_cleanup_started", connection_id = conn_key, total_messages_processed = message_count);
	info!("Cleaning up connection {} after {} messages", conn_key, message_count);

	let cleanup_result = health_check!("connection_cleanup", { state.remove_connection(conn_key, "Connection closed".to_string()).await });

	if let Err(e) = cleanup_result {
		record_connection_error!("cleanup_failed", "cleanup", e);
		error!("Failed to remove connection {}: {}", conn_key, e);
	}

	forward_task.abort();
	record_system_event!("websocket_cleanup_completed", connection_id = conn_key);
	info!("Connection {} cleanup completed", conn_key);
}

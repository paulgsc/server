use super::*;
use async_broadcast::{broadcast, Receiver, Sender};
use std::collections::HashSet;
use tokio::time::{Duration, Instant};
use tracing::{info, warn};
// Connection FSM container with proper resource management
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
		sender.set_await_active(false); // Prevent blocking on slow clients
		sender.set_overflow(true);

		let mut subscriptions = HashSet::new();
		subscriptions.insert(EventType::Ping);
		subscriptions.insert(EventType::Pong);
		subscriptions.insert(EventType::Error);
		subscriptions.insert(EventType::ClientCount); // Always subscribe to client count

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
	// Enhanced connection management with proper resource tracking
	pub async fn add_connection(&self) -> Result<(String, Receiver<Event>), String> {
		let (connection, receiver) = Connection::new();
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

	pub async fn remove_connection(&self, client_key: &str, reason: String) -> Result<(), String> {
		if let Some((_, mut connection)) = self.connections.remove(client_key) {
			let connection_id = connection.id.clone();
			let was_active = connection.is_active();
			let duration = connection.established_at.elapsed();

			// Transition to disconnected state
			let old_state = connection.disconnect(reason.clone())?;
			self.metrics.connection_removed(was_active);

			// Use comprehensive cleanup macro
			cleanup_connection!(connection_id, &reason, duration, true);
			update_resource_usage!("active_connections", self.connections.len() as f64);

			// Emit system events
			record_system_event!(
				"connection_state_changed",
				connection_id = connection_id,
				from_state = old_state,
				to_state = connection.state
			);

			info!("Connection {} removed: {}", connection_id, reason);

			// Update and broadcast client count
			self.broadcast_client_count().await;
		}
		Ok(())
	}

	// Enhanced timeout monitor with invariant checking
	pub fn start_timeout_monitor(&self, timeout: Duration) {
		let connections = self.connections.clone();
		let metrics = self.metrics.clone();
		let sender = self.sender.clone();

		tokio::spawn(async move {
			let mut interval = tokio::time::interval(Duration::from_secs(30));

			loop {
				interval.tick().await;

				// Comprehensive health check with instrumentation
				let health_result: Result<(), String> = health_check!("timeout_monitor", {
					let total_connections = connections.len();
					let metrics_snapshot = metrics.get_snapshot();

					// Check invariants
					let expected_active = metrics_snapshot.total_created - metrics_snapshot.total_removed;
					check_invariant!(
					total_connections as u64 == expected_active,
											"connection_count",
																	"Connection count mismatch",
																							expected: expected_active,
																													actual: total_connections as u64
																																		);

					// Log comprehensive health snapshot
					log_health_snapshot!(metrics, total_connections);

					Ok(())
				});

				if health_result.is_err() {
					record_ws_error!("health_check_failed", "timeout_monitor");
				}

				// Find and process stale connections
				let stale_connection_keys: Vec<String> = connections
					.iter()
					.filter_map(|entry| {
						let conn = entry.value();
						if conn.is_stale(timeout) {
							Some(entry.key().clone())
						} else {
							None
						}
					})
					.collect();

				// Process stale connections with instrumentation
				let mut cleaned_up = 0;
				for client_key in stale_connection_keys {
					if let Some(mut entry) = connections.get_mut(&client_key) {
						let conn = entry.value_mut();
						let connection_id = conn.id.clone();

						if let Ok(old_state) = conn.mark_stale("Timeout".to_string()) {
							update_connection_state!("active", "stale");
							metrics.connection_marked_stale();

							record_system_event!("connection_state_changed", connection_id = connection_id, from_state = old_state, to_state = conn.state);

							warn!("Connection {} marked as stale due to timeout", connection_id);
						}
					}

					// Remove stale connection with cleanup instrumentation
					if let Some((_, mut conn)) = connections.remove(&client_key) {
						let connection_id = conn.id.clone();
						let duration = conn.established_at.elapsed();
						let _ = conn.disconnect("Timeout cleanup".to_string());
						cleaned_up += 1;

						cleanup_connection!(connection_id, "Timeout cleanup", duration, true);
					}
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

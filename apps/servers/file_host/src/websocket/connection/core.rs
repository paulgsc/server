use super::errors::ConnectionError;
use crate::websocket::EventType;
use crate::*;
use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use futures::sink::SinkExt;
use futures::stream::SplitSink;
use some_transport::Transport;
use std::net::SocketAddr;
use tokio::{task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_connection::{ClientId, Connection, ConnectionState};

// Connection management operations
impl WebSocketFsm {
	/// Generate a ClientId from request headers and socket address
	pub fn client_id_from_request(&self, headers: &HeaderMap, addr: &SocketAddr) -> ClientId {
		// Priority order:
		// 1. X-Client-ID
		if let Some(client_id) = headers.get("x-client-id").and_then(|v| v.to_str().ok()) {
			if !client_id.is_empty() && client_id.len() <= 64 {
				return ClientId::new(format!("auth:{}", client_id));
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
				return ClientId::new(format!("proxy:{}:{:x}", client_ip, user_agent_hash));
			}
		}

		// Fallback: direct IP + user agent hash
		ClientId::new(format!("direct:{}:{:x}", addr.ip(), user_agent_hash))
	}

	/// Adds a connection to the store with comprehensive observability
	pub async fn add_connection(&self, headers: &HeaderMap, addr: &SocketAddr, cancel_token: &CancellationToken) -> Result<(String, ConnectionReceivers), ConnectionError> {
		let start = Instant::now();
		let client_id = self.client_id_from_request(headers, addr);

		let domain_conn = Connection::new(client_id.clone(), *addr);

		let connection_id = domain_conn.id.clone();
		let client_key = connection_id.as_string();

		// Default subscriptions that all connections get
		let default_subs = vec![EventType::Ping, EventType::Pong, EventType::Error, EventType::ClientCount];

		// Subscribe to NATS subjects for default event types
		let receivers = self
			.subscribe_connection(&client_key, default_subs.clone())
			.await
			.map_err(|e| ConnectionError::SubscriptionFailed(e))?;

		let handle = self.store.insert(client_key.clone(), domain_conn, cancel_token);

		// Update the actor's subscription state to match
		handle.subscribe(default_subs).await.map_err(|e| ConnectionError::SubscriptionFailed(e))?;
		let elapsed = start.elapsed();

		self.metrics.connection_created();

		record_connection_created!(connection_id, client_id);
		info!(
			connection_id = %connection_id,
			client_id = %client_id,
			addr = %addr,
			setup_duration_ms = elapsed.as_millis(),
			"Connection added successfully"
		);

		let to = handle.get_state().await.map_err(|e| ConnectionError::StateRetrievalFailed(e.to_string()))?;

		// Emit system event
		let system_event = Event::system(SystemEvent {
			event_type: "connection_established".to_owned(),
			payload: serde_json::to_vec(&serde_json::json!({
				"connection_id": connection_id.to_owned(),
				"client_id": client_id.to_owned(),
			}))
			.unwrap_or_default(),
		});

		let _ = self.broadcast_event(system_event).await;

		Ok((client_key, receiver))
	}

	/// Get connections by client ID with observability
	pub async fn get_client_connections(&self, client_id: &ClientId) -> Vec<String> {
		let start = Instant::now();

		let connections: Vec<String> = self
			.store
			.keys()
			.into_iter()
			.filter(|key| {
				if let Some(handle) = self.store.get(key) {
					&handle.connection.client_id == client_id
				} else {
					false
				}
			})
			.collect();

		let elapsed = start.elapsed();

		if !connections.is_empty() {
			info!(
				client_id = %client_id,
				connection_count = connections.len(),
				query_duration_ms = elapsed.as_millis(),
				"Retrieved client connections"
			);
		}

		connections
	}

	/// Remove a connection with comprehensive cleanup and observability
	pub async fn remove_connection(&self, client_key: &str, reason: String) -> Result<(), ConnectionError> {
		let start = Instant::now();

		match self.store.remove(client_key).await {
			Some(handle) => {
				let connection_id = handle.connection.id.clone();
				let client_id = handle.connection.client_id.clone();
				let duration = handle.connection.get_duration();

				let state = handle.get_state().await.ok();
				let was_active = state.as_ref().map(|s| s.is_active).unwrap_or(false);

				if let Err(e) = handle.shutdown().await {
					warn!(
						connection_id = %connection_id,
						error = %e,
						"Failed to gracefully shutdown connection actor"
					);
				}

				self
					.transport
					.close_channel(client_key)
					.await
					.map_err(|e| ConnectionError::TransportCloseFailed(e.to_string()))?;

				self.metrics.connection_removed(was_active);

				let elapsed = start.elapsed();

				record_connection_removed!(connection_id, client_id, duration, reason);
				info!(
					connection_id = %connection_id,
					client_id = %client_id,
					lifetime_ms = duration.as_millis(),
					was_active = was_active,
					reason = %reason,
					cleanup_duration_ms = elapsed.as_millis(),
					"Connection removed"
				);

				self.broadcast_client_count().await;

				// Emit system event
				let system_event = Event::system(SystemEvent {
					event_type: "connection_cleanup".to_owned(),
					payload: serde_json::to_vec(&serde_json::json!({
						"connection_id": connection_id.to_owned(),
						"reason": reason,
						"lifetime_ms": duration.as_millis(),
					}))
					.unwrap_or_default(),
				});

				let _ = self.broadcast_event(system_event).await;

				Ok(())
			}
			None => {
				warn!(
					connection_key = client_key,
					reason = %reason,
					"Attempted to remove non-existent connection"
				);
				Ok(())
			}
		}
	}

	/// Handle subscription changes for a connection
	pub async fn handle_subscription_update(
		&self,
		connection_id: &str,
		add_types: Vec<EventType>,
		remove_types: Vec<EventType>,
		receivers: &ConnectionReceivers,
	) -> Result<(), ConnectionError> {
		// Update NATS subscriptions
		self
			.update_subscriptions(connection_id, add_types.clone(), remove_types.clone(), receivers)
			.await
			.map_err(|e| ConnectionError::SubscriptionFailed(e))?;

		// Update actor subscription state
		if let Some(handle) = self.store.get(connection_id) {
			if !add_types.is_empty() {
				handle.subscribe(add_types).await.map_err(|e| ConnectionError::SubscriptionFailed(e.to_owned()))?;
			}

			if !remove_types.is_empty() {
				handle.unsubscribe(remove_types).await.map_err(|e| ConnectionError::SubscriptionFailed(e.to_owned()))?;
			}
		}

		Ok(())
	}
}

// ===== Helper functions for WebSocket lifecycle =====

pub(crate) async fn establish_connection(
	state: &WebSocketFsm,
	headers: &HeaderMap,
	addr: &SocketAddr,
	cancel_token: &CancellationToken,
) -> Result<(String, ConnectionReceivers), ConnectionError> {
	let (key, receivers) = state.add_connection(headers, addr, cancel_token).await?;
	record_system_event!("websocket_established", connection_id = key);
	info!(connection_id = %key, "WebSocket connection established");
	Ok((key, receivers))
}

pub(crate) async fn send_initial_handshake(sender: &mut SplitSink<WebSocket, Message>) -> Result<(), ConnectionError> {
	let ping_event = Event::Ping;
	let msg = serde_json::to_string(&ping_event)?;

	sender.send(Message::Text(msg)).await.map_err(|e| ConnectionError::HandshakeFailed(e.to_string()))?;

	Ok(())
}

pub(crate) async fn clear_connection(state: &WebSocketFsm, conn_key: &str) {
	let cleanup_result = health_check!("connection_cleanup", {
		state.remove_connection(conn_key, "Connection failed during setup".to_string()).await
	});

	if let Err(e) = cleanup_result {
		record_connection_error!("cleanup_failed", "cleanup", e);
		error!(
			connection_id = %conn_key,
			error = %e,
			"Failed to remove connection during cleanup"
		);
	}
}

pub(crate) async fn cleanup_connection_with_stats(state: &WebSocketFsm, conn_key: &str, message_count: u64, forward_task: JoinHandle<()>) {
	record_system_event!("websocket_cleanup_started", connection_id = conn_key, total_messages_processed = message_count);

	info!(
		connection_id = %conn_key,
		messages_processed = message_count,
		"Starting connection cleanup"
	);

	let cleanup_result = health_check!("connection_cleanup", { state.remove_connection(conn_key, "Connection closed".to_string()).await });

	if let Err(e) = cleanup_result {
		record_connection_error!("cleanup_failed", "cleanup", e);
		error!(
			connection_id = %conn_key,
			error = %e,
			"Failed to remove connection during cleanup"
		);
	}

	forward_task.abort();

	record_system_event!("websocket_cleanup_completed", connection_id = conn_key);
	info!(
		connection_id = %conn_key,
		"Connection cleanup completed"
	);
}

use crate::websocket::EventType;
use crate::*;
use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use futures::sink::SinkExt;
use futures::stream::SplitSink;
use some_transport::{InMemTransportReceiver, Transport};
use std::net::SocketAddr;
use tokio::{task::JoinHandle, time::Instant};
use tracing::{error, info, warn};
use ws_connection::{ClientId, Connection, ConnectionState};

// Infrastructure extensions for domain Connection
pub trait ConnectionExt {
	fn initialize_default_subscriptions(&mut self);
}

impl ConnectionExt for Connection<EventType> {
	fn initialize_default_subscriptions(&mut self) {
		// Add default subscriptions that all connections need
		self.subscribe(vec![EventType::Ping, EventType::Pong, EventType::Error, EventType::ClientCount]);
	}
}

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
	pub async fn add_connection(&self, headers: &HeaderMap, addr: &SocketAddr) -> Result<(String, InMemTransportReceiver<Event>), String> {
		let start = Instant::now();
		let client_id = self.client_id_from_request(headers, addr);

		let mut domain_conn = Connection::new(client_id.clone(), *addr);
		domain_conn.initialize_default_subscriptions();

		let connection_id = domain_conn.id.clone();
		let client_key = connection_id.as_string();

		let receiver = self.transport.open_channel(&client_key).await;

		let handle = self.store.insert(client_key.clone(), domain_conn);
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

		let to = handle.get_state().await.map_err(|e| format!("Failed to get connection state: {}", e))?;
		self
			.emit_system_event(Event::ConnectionStateChanged {
				connection_id: connection_id.clone(),
				from: ConnectionState::new(),
				to,
			})
			.await;

		self.subscriber_notify.notify_waiters();

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
	pub async fn remove_connection(&self, client_key: &str, reason: String) -> Result<(), String> {
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

				self.transport.close_channel(client_key).await.map_err(|e| e.to_string())?;
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

				// Emit system event
				self
					.emit_system_event(Event::ConnectionCleanup {
						connection_id: connection_id.clone(),
						reason: reason.clone(),
						resources_freed: true,
					})
					.await;

				self.broadcast_client_count().await;

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
}

// ===== Helper functions for WebSocket lifecycle =====

pub(crate) async fn establish_connection(state: &WebSocketFsm, headers: &HeaderMap, addr: &SocketAddr) -> Result<(String, InMemTransportReceiver<Event>), ()> {
	match state.add_connection(headers, addr).await {
		Ok((key, rx)) => {
			record_system_event!("websocket_established", connection_id = key);
			info!(connection_id = %key, "WebSocket connection established");
			Ok((key, rx))
		}
		Err(e) => {
			record_connection_error!("creation_failed", "creation", e);
			error!(error = %e, "Failed to add connection");
			Err(())
		}
	}
}

pub(crate) async fn send_initial_handshake(sender: &mut SplitSink<WebSocket, Message>, conn_key: &str) -> Result<(), ()> {
	let ping_event = Event::Ping;
	match serde_json::to_string(&ping_event) {
		Ok(msg) => {
			if let Err(e) = sender.send(Message::Text(msg)).await {
				record_connection_error!("handshake_failed", "creation", e);
				error!(
					connection_id = %conn_key,
					error = %e,
					"Failed to send initial ping"
				);
				return Err(());
			}
			Ok(())
		}
		Err(e) => {
			record_connection_error!("handshake_serialization_failed", "creation", e);
			error!(
				connection_id = %conn_key,
				error = %e,
				"Failed to serialize initial ping"
			);
			Err(())
		}
	}
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

use crate::WebSocketFsm;
use axum::http::HeaderMap;
use std::net::SocketAddr;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use ws_connection::{ClientId, Connection};
use ws_events::events::EventType;

pub(crate) mod errors;
pub(crate) mod handlers;
pub mod instrument;

use errors::ConnectionError;
pub(crate) use handlers::{clear_connection, establish_connection, send_initial_handshake};

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
	pub async fn add_connection(&self, headers: &HeaderMap, addr: &SocketAddr, cancel_token: &CancellationToken) -> Result<String, ConnectionError> {
		let start = Instant::now();
		let client_id = self.client_id_from_request(headers, addr);

		let domain_conn = Connection::new(client_id.clone(), *addr);

		let connection_id = domain_conn.id.clone();
		let client_key = connection_id.as_string();

		// Default subscriptions that all connections get
		let default_subs = vec![EventType::Ping, EventType::Pong, EventType::Error, EventType::ClientCount];

		let handle = self.store.insert(client_key.clone(), domain_conn, cancel_token);

		// Update the actor's subscription state to match
		handle.subscribe(default_subs).await.map_err(|e| ConnectionError::SubscriptionFailed(e))?;
		let elapsed = start.elapsed();

		info!(
			connection_id = %connection_id,
			client_id = %client_id,
			addr = %addr,
			setup_duration_ms = elapsed.as_millis(),
			"Connection added successfully"
		);

		Ok(client_key)
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

				let elapsed = start.elapsed();

				info!(
					connection_id = %connection_id,
					client_id = %client_id,
					lifetime_ms = duration.as_millis(),
					was_active = was_active,
					reason = %reason,
					cleanup_duration_ms = elapsed.as_millis(),
					"Connection removed"
				);

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
	pub async fn handle_subscription_update(&self, connection_id: &str, add_types: Vec<EventType>, remove_types: Vec<EventType>) -> Result<(), ConnectionError> {
		// Update actor subscription state
		if let Some(handle) = self.store.get(connection_id) {
			if !add_types.is_empty() {
				handle.subscribe(add_types).await.map_err(|e| ConnectionError::SubscriptionFailed(e))?;
			}

			if !remove_types.is_empty() {
				handle.unsubscribe(remove_types).await.map_err(|e| ConnectionError::SubscriptionFailed(e))?;
			}
		}

		Ok(())
	}
}

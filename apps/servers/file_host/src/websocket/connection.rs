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

/// `client_type` label for [`instrument::record_created`]/[`instrument::record_removed`]
/// and `ConnectionCleanup`'s own decrement in `websocket.rs` — the same
/// `probe:`/`auth:`/`proxy:`/`direct:` prefix convention `client_id_from_request`
/// writes, read back out. A free function rather than a method so
/// `websocket.rs` can derive the label without going through the store.
pub(crate) fn client_type_label(client_id: &ClientId) -> &'static str {
	if client_id.as_str().starts_with("probe:") {
		"probe"
	} else if client_id.as_str().starts_with("auth:") {
		"auth"
	} else if client_id.as_str().starts_with("proxy:") {
		"proxy"
	} else {
		"direct"
	}
}

// Connection management operations
impl WebSocketFsm {
	/// Generate a ClientId from request headers and socket address
	pub fn client_id_from_request(&self, headers: &HeaderMap, addr: &SocketAddr) -> ClientId {
		// 0. The blackbox WS liveness probe (infra/blackbox.yml's
		// `websocket_blackbox_http` job) completes a real upgrade against
		// `/ws` on every scrape and then hangs up without ever sending a
		// frame — self-identified via this header so it lands in
		// `client_type="probe"` rather than being counted as a device in
		// WS CONNS. One fixed id rather than per-request uniqueness: it is
		// monitoring infrastructure, not a client worth distinguishing
		// instances of.
		if headers.get("x-probe-source").is_some() {
			return ClientId::new("probe:blackbox");
		}

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
		let client_type = client_type_label(&client_id);

		let domain_conn = Connection::new(client_id.clone(), *addr);

		let connection_id = domain_conn.id.clone();
		let client_key = connection_id.as_string();

		// Default subscriptions that all connections get
		let default_subs = vec![EventType::Ping, EventType::Pong, EventType::Error, EventType::ClientCount];

		let handle = self.store.insert(client_key.clone(), domain_conn, cancel_token);

		// The entry occupies a store slot from this point regardless of
		// whether the subscribe below succeeds, so `created`/`connected` are
		// recorded here rather than after — a subscribe failure that leaves
		// this entry stranded (see the `?` below: nothing removes it on this
		// path today) should show up as `connected` growing, not disappear
		// from the count entirely.
		instrument::record_created(client_type);
		instrument::set_connected(self.store.len());

		// Update the actor's subscription state to match
		handle.subscribe(default_subs).await.map_err(|e| {
			instrument::record_error("subscription_failed", "creation");
			ConnectionError::SubscriptionFailed(e)
		})?;
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

				// `ws_connection_lifecycle_total{event="removed"}` and the
				// duration histogram are recorded once per socket at
				// `ConnectionCleanup::drop` (websocket.rs), not here — this
				// function is called from several places that Drop still
				// runs after (stale timeout, client close, forwarder end),
				// and double-instrumenting both would double-count. Only
				// `connected` — cheap, and this store entry genuinely just
				// left — is refreshed here for immediacy.
				instrument::set_connected(self.store.len());

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

#[cfg(test)]
mod tests {
	use super::*;

	fn addr() -> SocketAddr {
		"127.0.0.1:9999".parse().expect("valid socket addr")
	}

	/// The blackbox WS liveness probe's self-identifying header must win
	/// over every other branch, including `X-Client-ID` — a probe hitting
	/// `/ws` should never be able to land in `client_type="auth"` just
	/// because it also happens to carry a stray header.
	#[test]
	fn a_probe_header_is_tagged_probe_regardless_of_other_headers() {
		let fsm = WebSocketFsm::new();
		let mut headers = HeaderMap::new();
		headers.insert("x-probe-source", "blackbox-exporter".parse().unwrap());
		headers.insert("x-client-id", "someone".parse().unwrap());

		let client_id = fsm.client_id_from_request(&headers, &addr());
		assert!(client_id.as_str().starts_with("probe:"), "got {client_id:?}");
		assert_eq!(client_type_label(&client_id), "probe");
	}

	#[test]
	fn a_real_client_id_without_the_probe_header_is_tagged_auth() {
		let fsm = WebSocketFsm::new();
		let mut headers = HeaderMap::new();
		headers.insert("x-client-id", "someone".parse().unwrap());

		let client_id = fsm.client_id_from_request(&headers, &addr());
		assert_eq!(client_type_label(&client_id), "auth");
	}
}

use crate::{net::PeerKey, WebSocketFsm};
use axum::http::HeaderMap;
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
/// `probe:`/`proxy:`/`direct:` prefix convention `client_id_from_request`
/// writes, read back out. A free function rather than a method so
/// `websocket.rs` can derive the label without going through the store.
///
/// There is no `auth` label any more: it came from a client-supplied
/// `X-Client-ID` header that nothing authenticated (#372).
pub(crate) fn client_type_label(client_id: &ClientId) -> &'static str {
	if client_id.as_str().starts_with("probe:") {
		"probe"
	} else if client_id.as_str().starts_with("proxy:") {
		"proxy"
	} else {
		"direct"
	}
}

// Connection management operations
impl WebSocketFsm {
	/// Which client a connection belongs to, for grouping and metric labels.
	///
	/// A `ClientId` is **not an identity** (#372): it is derived from the
	/// peer's keyed, daily-rotating [`PeerKey`](crate::net::PeerKey), never
	/// from the raw address, and never from anything the client asserts
	/// about itself. Identity is `SubjectId`'s alone — see
	/// `docs/identity.md`. The user-agent hash this used to mix in is gone
	/// too: reading a fingerprinting header to tell two browsers on one
	/// address apart is exactly the tracking the privacy invariants rule out,
	/// and nothing reads `ClientId` finely enough to need it.
	#[must_use]
	pub fn client_id_from_request(&self, headers: &HeaderMap, peer: &PeerKey) -> ClientId {
		// The blackbox WS liveness probe (infra/blackbox.yml's
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

		// Behind a proxy every socket peer is the proxy; the first forwarded
		// hop is the only thing that tells its clients apart.
		if let Some(forwarded) = crate::net::forwarded_peer_key(headers) {
			return ClientId::new(String::from("proxy:") + forwarded.as_str());
		}

		ClientId::new(String::from("direct:") + peer.as_str())
	}

	/// Adds a connection to the store with comprehensive observability
	///
	/// # Errors
	///
	/// Returns `ConnectionError::SubscriptionFailed` when the new connection's
	/// actor can't take its default subscriptions; the store entry is removed
	/// again before returning, so nothing strands.
	pub async fn add_connection(&self, headers: &HeaderMap, peer: &PeerKey, cancel_token: &CancellationToken) -> Result<String, ConnectionError> {
		let start = Instant::now();
		let client_id = self.client_id_from_request(headers, peer);
		let client_type = client_type_label(&client_id);

		let domain_conn = Connection::new(client_id.clone());

		let connection_id = domain_conn.id.clone();
		let client_key = connection_id.as_string();

		// Default subscriptions that all connections get
		let default_subs = vec![EventType::Ping, EventType::Pong, EventType::Error, EventType::ClientCount];

		let handle = self.store.insert(client_key.clone(), domain_conn, cancel_token);

		// The entry occupies a store slot from this point regardless of
		// whether the subscribe below succeeds, so `created`/`connected` are
		// recorded here rather than after.
		instrument::record_created(client_type);
		instrument::set_connected(self.store.len());

		// Update the actor's subscription state to match. On failure the
		// caller never gets a key, so no `ConnectionCleanup` will ever exist
		// to remove this entry — undo it here, or it strands in the store
		// (and in `connected`) until process exit.
		if let Err(e) = handle.subscribe(default_subs).await {
			instrument::record_error("subscription_failed", "creation");
			self.store.remove(&client_key).await;
			instrument::record_removed(client_type, "error", 0.0);
			instrument::set_connected(self.store.len());
			return Err(ConnectionError::SubscriptionFailed(e));
		}
		let elapsed = start.elapsed();

		info!(
			connection_id = %connection_id,
			client_id = %client_id,
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
	use crate::net::peer_key;

	fn peer() -> PeerKey {
		peer_key("127.0.0.1:9999".parse().unwrap())
	}

	/// The blackbox WS liveness probe's self-identifying header must win
	/// over every other branch — a probe hitting `/ws` through a proxy should
	/// still land in `client_type="probe"`, not be counted as a device.
	#[test]
	fn a_probe_header_is_tagged_probe_regardless_of_other_headers() {
		let fsm = WebSocketFsm::new();
		let mut headers = HeaderMap::new();
		headers.insert("x-probe-source", "blackbox-exporter".parse().unwrap());
		headers.insert("x-forwarded-for", "10.0.0.9".parse().unwrap());

		let client_id = fsm.client_id_from_request(&headers, &peer());
		assert!(client_id.as_str().starts_with("probe:"), "got {client_id:?}");
		assert_eq!(client_type_label(&client_id), "probe");
	}

	/// #372: nothing a client asserts about itself becomes its `ClientId`.
	/// `X-Client-ID` used to be read verbatim and labelled `auth`, although
	/// nothing authenticated it.
	#[test]
	fn a_self_asserted_client_id_header_is_ignored() {
		let fsm = WebSocketFsm::new();
		let mut headers = HeaderMap::new();
		headers.insert("x-client-id", "someone".parse().unwrap());

		let client_id = fsm.client_id_from_request(&headers, &peer());
		assert_eq!(client_id, fsm.client_id_from_request(&HeaderMap::new(), &peer()));
		assert!(!client_id.as_str().contains("someone"), "got {client_id:?}");
		assert_eq!(client_type_label(&client_id), "direct");
	}

	#[test]
	fn a_forwarded_client_is_tagged_proxy_and_keyed_by_its_first_hop() {
		let fsm = WebSocketFsm::new();
		let mut headers = HeaderMap::new();
		headers.insert("x-forwarded-for", "10.0.0.9, 172.16.0.1".parse().unwrap());

		let client_id = fsm.client_id_from_request(&headers, &peer());
		assert_eq!(client_type_label(&client_id), "proxy");
		assert_eq!(client_id.as_str(), String::from("proxy:") + peer_key("10.0.0.9:1".parse().unwrap()).as_str());
	}

	/// Neither the socket peer's address nor a forwarded one survives into
	/// the `ClientId`, which is logged on every connection.
	#[test]
	fn no_client_id_contains_an_address() {
		let fsm = WebSocketFsm::new();
		let mut forwarded = HeaderMap::new();
		forwarded.insert("x-forwarded-for", "10.0.0.9".parse().unwrap());

		for client_id in [fsm.client_id_from_request(&HeaderMap::new(), &peer()), fsm.client_id_from_request(&forwarded, &peer())] {
			assert!(!client_id.as_str().contains("127.0.0.1"), "got {client_id:?}");
			assert!(!client_id.as_str().contains("10.0.0.9"), "got {client_id:?}");
		}
	}
}

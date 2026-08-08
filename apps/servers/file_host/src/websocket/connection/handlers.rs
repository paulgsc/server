use super::errors::ConnectionError;
use crate::WebSocketFsm;
use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use futures::sink::SinkExt;
use futures::stream::SplitSink;
use std::net::SocketAddr;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use ws_events::events::Event;

pub(crate) async fn establish_connection(state: &WebSocketFsm, headers: &HeaderMap, addr: &SocketAddr, cancel_token: &CancellationToken) -> Result<String, ConnectionError> {
	let key = state.add_connection(headers, addr, cancel_token).await?;
	info!(connection_id = %key, "WebSocket connection established");
	Ok(key)
}

pub(crate) async fn send_initial_handshake(sender: &mut SplitSink<WebSocket, Message>) -> Result<(), ConnectionError> {
	let ping_event = Event::Ping;
	let msg = serde_json::to_string(&ping_event)?;

	sender.send(Message::Text(msg)).await.map_err(|e| ConnectionError::HandshakeFailed(e.to_string()))?;

	Ok(())
}

/// Removes a connection that was added to the store but will never get a
/// `ConnectionCleanup` (`websocket.rs`) — the handshake send failed before
/// that guard was ever constructed, so nothing else will record its removal.
/// `client_type` is passed in by the caller (recomputed from the same
/// headers/addr `add_connection` used) rather than looked up here, since the
/// store entry is gone by the time this returns.
pub(crate) async fn clear_connection(state: &WebSocketFsm, conn_key: &str, client_type: &'static str) {
	let result = state.remove_connection(conn_key, "Connection failed during setup".to_string()).await;
	super::instrument::record_removed(client_type, "error", 0.0);

	if let Err(e) = result {
		error!(
			connection_id = %conn_key,
			error = %e,
			"Failed to remove connection during cleanup"
		);
	}
}

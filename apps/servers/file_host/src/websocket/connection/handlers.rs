use super::errors::ConnectionError;
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
use ws_events::events::EventType;

pub(crate) async fn establish_connection(state: &WebSocketFsm, headers: &HeaderMap, addr: &SocketAddr, cancel_token: &CancellationToken) -> Result<String, ConnectionError> {
	let key = state.add_connection(headers, addr, cancel_token).await?;
	record_system_event!("websocket_established", connection_id = key);
	info!(connection_id = %key, "WebSocket connection established");
	Ok(key)
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

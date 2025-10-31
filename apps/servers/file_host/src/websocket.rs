use crate::*;
use axum::{
	extract::{
		ws::{WebSocket, WebSocketUpgrade},
		ConnectInfo, FromRef, State,
	},
	http::{HeaderMap, StatusCode},
	response::IntoResponse,
	routing::get,
	Router,
};
use futures::stream::StreamExt;
use std::{net::SocketAddr, sync::Arc};
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_connection::ConnectionStore;
use ws_events::events::EventType;

pub mod broadcast;
pub mod connection;
pub mod heartbeat;
pub mod message;
pub mod shutdown;

use broadcast::spawn_event_forwarder;
use connection::{cleanup_connection_with_stats, clear_connection, establish_connection, send_initial_handshake};
use message::process_incoming_messages;

// Enhanced WebSocket FSM with comprehensive observability
#[derive(Clone)]
pub struct WebSocketFsm {
	/// Domain layer: Connection actor handles
	store: Arc<ConnectionStore<EventType>>,
}

impl WebSocketFsm {
	/// Creates a new WebSocketFsm instance - only responsible for initialization
	pub fn new() -> Self {
		let store = Arc::new(ConnectionStore::<EventType>::new());
		Self { store }
	}

	pub fn router<S>(self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
		AppState: FromRef<S>,
	{
		Router::new().route("/ws", get(websocket_handler))
	}
}

async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> impl IntoResponse {
	let client_id = addr.ip().to_string();
	let cancel_token = state.core.cancel_token.clone();
	info!("Incoming WS request from {client_id}");

	if !state.core.connection_guard.try_acquire_permit_hint() {
		warn!("Global limit exceeded — rejecting early");
		return (StatusCode::SERVICE_UNAVAILABLE, "Too many connections").into_response();
	}

	// Wrap acquire in a timeout (e.g., 5 seconds)
	match timeout(Duration::from_secs(5), state.core.connection_guard.acquire(client_id.clone())).await {
		Ok(Ok(permit)) => ws.on_upgrade(move |socket| handle_socket(socket, state, headers, addr, permit, cancel_token)),
		Ok(Err(err)) => {
			use AcquireErrorKind::*;
			let reason = match err.kind {
				QueueFull => "Too many pending connections for this client",
				GlobalLimit => "Server is at capacity",
			};
			error!("Rejecting WS for {client_id}: {reason}");
			(StatusCode::SERVICE_UNAVAILABLE, reason).into_response()
		}
		Err(_timeout_elapsed) => {
			error!("Timeout waiting for permit for {client_id}");
			(StatusCode::REQUEST_TIMEOUT, "Connection acquisition timed out").into_response()
		}
	}
}

/// Orchestrates the WebSocket connection lifecycle
async fn handle_socket(socket: WebSocket, state: AppState, headers: HeaderMap, addr: SocketAddr, permit: ConnectionPermit, cancel_token: CancellationToken) {
	let (mut sender, receiver) = socket.split();

	// Direct WS client channel (mpsc)
	let (ws_tx, ws_rx) = tokio::sync::mpsc::unbounded_channel();

	let transport = state.realtime.transport;
	let ws_fsm = state.realtime.ws;

	// Establish connection through FSM
	let conn_key = match establish_connection(&ws_fsm, &headers, &addr, &cancel_token).await {
		Ok(connection) => connection,
		Err(_) => {
			return;
		}
	};

	if send_initial_handshake(&mut sender).await.is_err() {
		clear_connection(&ws_fsm, &conn_key).await;
		return;
	}

	// Pass cancel token to both tasks
	let forward_cancel = cancel_token.child_token().clone();
	let process_cancel = cancel_token.child_token().clone();

	let forward_task = spawn_event_forwarder(sender, ws_rx, ws_fsm.clone(), transport.clone(), conn_key.clone(), forward_cancel);

	let message_count = process_incoming_messages(receiver, &ws_fsm, transport.clone(), ws_tx.clone(), &conn_key, process_cancel).await;

	cleanup_connection_with_stats(&ws_fsm, &conn_key, message_count, forward_task).await;
	permit.release();
}

// Re-export for compatibility
pub use WebSocketFsm as WebSocketState;

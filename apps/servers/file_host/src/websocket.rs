use crate::AppState;
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
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_conn_manager::{AcquireErrorKind, ConnectionPermit};
use ws_connection::ConnectionStore;
use ws_events::events::EventType;

pub mod broadcast;
pub mod connection;
pub mod heartbeat;
pub mod message;
pub mod shutdown;

use broadcast::spawn_event_forwarder;
use connection::{clear_connection, client_type_label, establish_connection, instrument, send_initial_handshake};
use message::spawn_process_incoming_messages;

/// How long a connection's message-processing loop will let it sit without
/// inbound activity before treating it as stale — see
/// `message::handlers::process_incoming_messages`. Also the "freshness"
/// window `connection_gauges` uses for `live`, read by the periodic gauge
/// sweep for the CLIENTS dashboard row (#214/#227). `nudge::presence` no
/// longer shares this window: presence moved to a client-written lease with
/// its own TTL (`NUDGE_PRESENCE_LEASE_TTL_SECONDS`), because a WebSocket
/// connection's staleness was never actually the same question as "does this
/// subject have a fresh presence lease" — the old code sharing one constant
/// for both was a coincidence of implementation, not a real invariant.
pub const STALE_TIMEOUT: Duration = Duration::from_secs(120);

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

	/// `connected` / `live` / `subscribed` — see #227. Awaits every
	/// connection actor, so this belongs on a periodic background sweep
	/// (`metrics::periodic`), never on the request or scrape path; `connected`
	/// alone is cheap and is also refreshed synchronously from
	/// `add_connection`/`remove_connection`.
	pub async fn connection_gauges(&self, freshness: Duration) -> ConnectionGaugeSnapshot {
		let keys = self.store.keys();
		let connected = keys.len();
		let mut live = 0;
		let mut subscribed = 0;

		for key in keys {
			let Some(handle) = self.store.get(&key) else { continue };

			// An actor that cannot answer is not evidence of anyone being
			// present, so a failed query counts as absence rather than
			// propagating — presence must never be able to fail *closed*.
			if let Ok(state) = handle.get_state().await {
				if state.is_active && !state.is_stale && state.last_activity.elapsed() <= freshness {
					live += 1;
				}
			}

			if let Ok(subs) = handle.get_subscriptions().await {
				if !subs.is_empty() {
					subscribed += 1;
				}
			}
		}

		ConnectionGaugeSnapshot { connected, live, subscribed }
	}
}

/// See [`WebSocketFsm::connection_gauges`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ConnectionGaugeSnapshot {
	pub connected: usize,
	pub live: usize,
	pub subscribed: usize,
}

struct ConnectionCleanup {
	forward_task: Option<JoinHandle<()>>,
	message_task: Option<JoinHandle<(u64, &'static str)>>,

	forward_cancel: CancellationToken,
	process_cancel: CancellationToken,

	permit: Option<ConnectionPermit>,

	/// The single choke point every socket's teardown passes through exactly
	/// once — see #226/#227 and this module's `instrument` doc comment.
	/// `client_type`/`started_at` are fixed at construction; `end_reason` is
	/// set by `handle_socket` right before this drops, from whichever
	/// `tokio::select!` branch actually explains the teardown.
	client_type: &'static str,
	started_at: Instant,
	end_reason: &'static str,
}

impl Drop for ConnectionCleanup {
	fn drop(&mut self) {
		self.forward_cancel.cancel();
		self.process_cancel.cancel();

		if let Some(task) = self.forward_task.take() {
			task.abort();
		}

		if let Some(task) = self.message_task.take() {
			task.abort();
		}

		if let Some(permit) = self.permit.take() {
			permit.release();
		}

		instrument::record_removed(self.client_type, self.end_reason, self.started_at.elapsed().as_secs_f64());
	}
}

async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> impl IntoResponse {
	let client_id = addr.ip().to_string();
	let cancel_token = state.core.cancel_token.clone();
	info!("Incoming WS request from {client_id}");

	if !state.core.connection_guard.try_acquire_permit_hint() {
		warn!("Global limit exceeded — rejecting early");
		crate::metrics::refusals::record_ws("global_capacity");
		return (StatusCode::SERVICE_UNAVAILABLE, "Too many connections").into_response();
	}

	// Wrap acquire in a timeout (e.g., 5 seconds)
	match timeout(Duration::from_secs(5), state.core.connection_guard.acquire(client_id.clone())).await {
		Ok(Ok(permit)) => ws.on_upgrade(move |socket| handle_socket(socket, state, headers, addr, permit, cancel_token)),
		Ok(Err(err)) => {
			use AcquireErrorKind::*;
			let (reason, metric_reason) = match err.kind {
				QueueFull => ("Too many pending connections for this client", "queue_full"),
				// A second, near-unreachable path to the same fast-path
				// hint's conclusion — it only fires if the global semaphore
				// is closed, which nothing in this codebase does today —
				// so it shares that reason rather than inventing a fourth.
				GlobalLimit => ("Server is at capacity", "global_capacity"),
			};
			error!("Rejecting WS for {client_id}: {reason}");
			crate::metrics::refusals::record_ws(metric_reason);
			(StatusCode::SERVICE_UNAVAILABLE, reason).into_response()
		}
		Err(_timeout_elapsed) => {
			error!("Timeout waiting for permit for {client_id}");
			crate::metrics::refusals::record_ws("permit_timeout");
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

	let client_type = client_type_label(&ws_fsm.client_id_from_request(&headers, &addr));

	if send_initial_handshake(&mut sender).await.is_err() {
		clear_connection(&ws_fsm, &conn_key, client_type).await;
		return;
	}

	// Pass cancel token to both tasks
	let forward_cancel = cancel_token.child_token().clone();
	let process_cancel = cancel_token.child_token().clone();

	let forward_task = spawn_event_forwarder(sender, ws_rx, ws_fsm.clone(), transport.clone(), conn_key.clone(), forward_cancel.clone());

	let message_task = spawn_process_incoming_messages(receiver, ws_fsm.clone(), transport.clone(), ws_tx.clone(), conn_key.clone(), process_cancel.clone());

	let mut cleanup = ConnectionCleanup {
		permit: Some(permit),
		forward_task: Some(forward_task),
		message_task: Some(message_task),
		forward_cancel,
		process_cancel,
		client_type,
		started_at: Instant::now(),
		end_reason: "cleanup",
	};

	// Which branch resolves first is *how* the connection ended; the actual
	// `end_reason` label comes from there, except cancellation always wins —
	// see below. `message_task`'s own reason (set by
	// `process_incoming_messages`) is the informative case; `forward_task`
	// ending on its own means the outbound side hit a send failure (the peer
	// is gone) or ran out of channels, both of which read as the client
	// having left.
	let resolved_reason: &'static str = tokio::select! {
		_ = cleanup.forward_task.as_mut().unwrap() => "client_disconnect",
		result = cleanup.message_task.as_mut().unwrap() => match result {
			Ok((_, reason)) => reason,
			Err(_) => "error", // task panicked or was aborted
		},
		_ = cancel_token.cancelled() => "cleanup",
	};

	// A server-shutdown cancellation racing one of the other two branches can
	// make either of them resolve "first" even though shutdown is why the
	// connection is really ending — `is_cancelled()` overrides whatever the
	// select above picked so that race can't mislabel a shutdown as a client
	// disconnect or an error.
	cleanup.end_reason = if cancel_token.is_cancelled() { "cleanup" } else { resolved_reason };
}

// Re-export for compatibility
pub use WebSocketFsm as WebSocketState;

#[cfg(test)]
mod tests {
	use super::*;
	use metrics_util::debugging::{DebugValue, DebuggingRecorder};
	use metrics_util::CompositeKey;

	type Snapshot = Vec<(CompositeKey, Option<metrics::Unit>, Option<metrics::SharedString>, DebugValue)>;

	fn find<'a>(snapshot: &'a Snapshot, name: &str, labels: &[(&str, &str)]) -> Option<&'a DebugValue> {
		snapshot.iter().find_map(|(key, _, _, value)| {
			let k = key.key();
			if k.name() != name {
				return None;
			}
			let matches = labels.iter().all(|(want_k, want_v)| k.labels().any(|l| l.key() == *want_k && l.value() == *want_v));
			matches.then_some(value)
		})
	}

	fn counter(snapshot: &Snapshot, name: &str, labels: &[(&str, &str)]) -> Option<u64> {
		match find(snapshot, name, labels) {
			Some(DebugValue::Counter(n)) => Some(*n),
			_ => None,
		}
	}

	fn gauge(snapshot: &Snapshot, name: &str, labels: &[(&str, &str)]) -> Option<f64> {
		match find(snapshot, name, labels) {
			Some(DebugValue::Gauge(n)) => Some(n.into_inner()),
			_ => None,
		}
	}

	/// #226 (C1)'s acceptance criterion: "opening a WebSocket and closing it
	/// moves `ws_connection_lifecycle_total{event="created"}` and
	/// `{event="removed"}`, and records a duration under a real
	/// `end_reason`." Exercised through `WebSocketFsm` and `ConnectionCleanup`
	/// directly rather than a real socket — those are the two seams that
	/// actually record the metrics (`add_connection` for `created`,
	/// `ConnectionCleanup::drop` for `removed`/duration — see this module's
	/// header comment on `instrument` for why they're split), and a live
	/// axum/tokio-tungstenite round trip would exercise a lot of unrelated
	/// machinery to reach the same two call sites.
	#[test]
	fn opening_and_closing_a_connection_moves_the_lifecycle_counters() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();

		metrics::with_local_recorder(&recorder, || {
			let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("build a current-thread runtime");

			rt.block_on(async {
				let fsm = WebSocketFsm::new();
				let headers = HeaderMap::new();
				let addr: SocketAddr = "127.0.0.1:9999".parse().expect("valid socket addr");
				let cancel = CancellationToken::new();

				// "Opening a WebSocket": the store-level half of connection
				// setup — the half that doesn't need a real TCP socket. Not
				// snapshotting here: `Snapshotter::snapshot()` swaps counter
				// and gauge storage back to zero as it reads it, so an
				// intermediate check here would consume the very count the
				// assertions below are looking for.
				let key = fsm.add_connection(&headers, &addr, &cancel).await.expect("add_connection");

				// "Closing it": the store entry goes first, the way every
				// real removal path in `message`/`broadcast` handlers calls
				// it before `ConnectionCleanup` ever drops.
				fsm.remove_connection(&key, "test teardown".to_string()).await.expect("remove_connection");

				// The socket-level half — built directly rather than through
				// `handle_socket`'s full upgrade dance, since `Drop` is the
				// only part of it this test needs.
				let cleanup = ConnectionCleanup {
					forward_task: Some(tokio::spawn(async {})),
					message_task: Some(tokio::spawn(async { (0u64, "client_disconnect") })),
					forward_cancel: CancellationToken::new(),
					process_cancel: CancellationToken::new(),
					permit: None,
					client_type: "direct",
					started_at: Instant::now(),
					end_reason: "client_disconnect",
				};
				drop(cleanup);
			});
		});

		let snapshot = snapshotter.snapshot().into_vec();

		assert_eq!(counter(&snapshot, "ws_connection_lifecycle_total", &[("event", "created")]), Some(1));
		assert_eq!(counter(&snapshot, "ws_connection_lifecycle_total", &[("event", "removed")]), Some(1));
		assert_eq!(gauge(&snapshot, "ws_connections", &[("state", "connected")]), Some(0.0));

		let DebugValue::Histogram(samples) =
			find(&snapshot, "ws_connection_duration_seconds", &[("end_reason", "client_disconnect")]).expect("duration recorded under a real end_reason")
		else {
			panic!("ws_connection_duration_seconds should be a histogram");
		};
		assert_eq!(samples.len(), 1);
	}
}

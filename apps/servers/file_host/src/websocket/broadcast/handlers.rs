use crate::WebSocketFsm;
use axum::extract::ws::{Message, WebSocket};
use futures::{sink::SinkExt, stream::SplitSink};
use some_transport::{NatsTransport, RecvResult, SendResult, SenderExt, Transport, UnboundedReceiverExt};
use tokio::{
	sync::mpsc::{self, UnboundedReceiver},
	time::{interval, Duration},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_events::{
	events::{Event, EventType},
	UnifiedEvent,
};

/// Spawn the NATS -> WS pipeline for a connection
pub(crate) fn spawn_event_forwarder(
	mut ws_sender: SplitSink<WebSocket, Message>,
	mut ws_direct: UnboundedReceiver<Event>,
	state: WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	conn_key: String,
	cancel_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		// send ping every 30s
		let mut ping_interval = interval(Duration::from_secs(30));
		// Per-event bounded channels
		let (obs_tx, mut obs_rx) = mpsc::channel::<Event>(10);
		let (tab_tx, mut tab_rx) = mpsc::channel::<Event>(100);
		let (utt_tx, mut utt_rx) = mpsc::channel::<Event>(100);
		let (orch_tx, mut orch_rx) = mpsc::channel::<Event>(100);

		// Spawn receiver tasks
		spawn_nats_task(EventType::ObsStatus, transport.clone(), obs_tx, conn_key.clone(), cancel_token.clone(), true);
		spawn_nats_task(EventType::TabMetaData, transport.clone(), tab_tx, conn_key.clone(), cancel_token.clone(), false);
		spawn_nats_task(EventType::Utterance, transport.clone(), utt_tx, conn_key.clone(), cancel_token.clone(), false);
		spawn_nats_task(EventType::OrchestratorState, transport.clone(), orch_tx, conn_key.clone(), cancel_token.clone(), false);

		let mut total_forwarded = 0u64;

		loop {
			tokio::select! {
				_ = cancel_token.cancelled() => {
					info!(connection_id=%conn_key, "Event forwarder cancelled");
					let _ = ws_sender.send(Message::Close(None)).await;
					break;
				}

				result = ws_direct.recv_graceful("ws_direct") => {
					match result {
						RecvResult::Message(msg) => {
							if forward_event(&mut ws_sender, &msg, &conn_key).await.is_ok() {
								total_forwarded += 1;
							}
						}
						RecvResult::SenderDropped => {
							debug!(connection_id=%conn_key, "Direct message sender dropped");
							continue;
						}
						RecvResult::Timeout => unreachable!("recv_graceful doesn't timeout"),
					}
				}

				Some(evt) = tab_rx.recv() => {
					if forward_event(&mut ws_sender, &evt, &conn_key).await.is_ok() {
						total_forwarded += 1;
					}
				}

				Some(evt) = utt_rx.recv() => {
					if forward_event(&mut ws_sender, &evt, &conn_key).await.is_ok() {
						total_forwarded += 1;
					}
				}

				Some(evt) = obs_rx.recv() => {
					if forward_event(&mut ws_sender, &evt, &conn_key).await.is_ok() {
						total_forwarded += 1;
					}
				}

				Some(evt) = orch_rx.recv() => {
					if forward_event(&mut ws_sender, &evt, &conn_key).await.is_ok() {
						total_forwarded += 1;
					}
				}
				// Send periodic pings to detect dead connections
				_ = ping_interval.tick() => {
					if let Err(e) = ws_sender.send(Message::Ping(vec![])).await {
						warn!("Failed to send ping to {conn_key}: {e} - client disconnected");
						break;
					}
					debug!("Sent ping to {conn_key}");
				}

				else => break, // all channels closed
			}
		}

		// Cleanup connection from store
		let _ = state.remove_connection(&conn_key, "Event forwarder ended".to_string()).await;

		info!(connection_id=%conn_key, total_forwarded, "Forwarding ended");
	})
}

/// Spawn a single NATS receiver task
fn spawn_nats_task(
	event_type: EventType,
	transport: NatsTransport<UnifiedEvent>,
	sender: mpsc::Sender<Event>,
	conn_key: String,
	cancel_token: CancellationToken,
	drop_if_full: bool,
) {
	tokio::spawn(async move {
		let mut rx = transport.subscribe_to_subject(&event_type.subject()).await;

		loop {
			tokio::select! {
				_ = cancel_token.cancelled() => break,

				result = rx.recv() => match result {
					Ok(unified) => {
						let event_result: Result<Event, String> = unified.into();
						let event = match event_result {
							Ok(e) => e,
							Err(e) => {
								warn!(connection_id=%conn_key, ?event_type, "Failed to convert UnifiedEvent: {}", e);
								continue;
							}
						};

						// Send using MPSC utils
						let send_result = if drop_if_full {
							sender.try_send_graceful(event.clone(), &format!("NATS {}", event_type.subject()))
						} else {
							sender.send_with_backpressure_warn(event.clone(), &format!("NATS {}", event_type.subject())).await
						};

						if let SendResult::ReceiverDropped(msg) = send_result {
							debug!(connection_id=%conn_key, ?event_type, "Receiver dropped, message lost: {:?}", msg);
						}
					}
					Err(e) => {
						error!(connection_id=%conn_key, ?event_type, "NATS receive error: {}", e);
						break;
					}
				}
			}
		}
	});
}

/// Forward a single event to the WebSocket client
async fn forward_event(sender: &mut SplitSink<WebSocket, Message>, event: &Event, conn_key: &str) -> Result<(), ()> {
	let json = serde_json::to_string(event).map_err(|e| {
		warn!(connection_id=%conn_key, "Failed to serialize event: {}", e);
	})?;

	sender.send(Message::Text(json)).await.map_err(|e| {
		warn!(connection_id=%conn_key, "Failed to forward WS message: {}", e);
	})
}

use crate::websocket::connection::instrument;
use crate::WebSocketFsm;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::{SplitStream, StreamExt};
use some_transport::NatsTransport;
use tokio::{
	sync::mpsc::UnboundedSender,
	task::JoinHandle,
	time::{interval, Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_events::events::{Event, UnifiedEvent};

pub(crate) fn spawn_process_incoming_messages(
	receiver: SplitStream<WebSocket>,
	state: WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	ws_tx: UnboundedSender<Event>,
	conn_key: String,
	cancel_token: CancellationToken,
) -> JoinHandle<(u64, &'static str)> {
	tokio::spawn(async move { process_incoming_messages(receiver, state, transport, ws_tx, conn_key, cancel_token).await })
}

/// Process all incoming messages from the WebSocket.
///
/// Returns `(messages_processed, end_reason)` — `end_reason` is one of
/// `timeout` | `client_disconnect` | `error` | `cleanup`, the same label set
/// `ConnectionCleanup::drop` (`websocket.rs`) records under, decided here
/// because this loop is the one place that actually knows *why* it broke.
async fn process_incoming_messages(
	mut receiver: SplitStream<WebSocket>,
	state: WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	ws_tx: UnboundedSender<Event>,
	conn_key: String,
	cancel_token: CancellationToken,
) -> (u64, &'static str) {
	let mut message_count = 0u64;

	let mut stale_check_interval = interval(Duration::from_secs(30));
	let stale_timeout = crate::websocket::STALE_TIMEOUT;

	let store = state.store.clone();
	let end_reason: &'static str;

	loop {
		tokio::select! {
			_ = cancel_token.cancelled() => {
				info!(
					connection_id = %conn_key,
					messages_processed = message_count,
					"WebSocket message processing cancelled - shutting down"
				);
				end_reason = "cleanup";
				break;
			}

			_ = stale_check_interval.tick() => {
				let Some(handle) = store.get(&conn_key) else {
					debug!(
						connection_id = %conn_key,
						"Connection actor missing during stale check - closing"
					);
					end_reason = "cleanup";
					break;
				};

				match handle.get_state().await {
					Ok(state_snapshot) => {
						let inactive = Instant::now()
							.duration_since(state_snapshot.last_activity);

						if inactive > stale_timeout {
							warn!(
								connection_id = %conn_key,
								inactive_seconds = inactive.as_secs(),
								"Connection is stale - closing"
							);

							let _ = state
								.remove_connection(
									&conn_key,
									"Stale connection - no inbound activity".to_string(),
								)
								.await;
							end_reason = "timeout";
							break;
						}
					}
					Err(e) => {
						warn!(
							connection_id = %conn_key,
							error = ?e,
							"Failed to fetch connection state - closing"
						);
						instrument::record_error("state_fetch_failed", "operation");
						end_reason = "error";
						break;
					}
				}
			}

			result = receiver.next() => {
				match result {
					Some(Ok(msg)) => {
						message_count += 1;

						let Some(handle) = store.get(&conn_key) else {
							debug!(
								connection_id = %conn_key,
								"Connection actor missing on inbound frame"
							);
							end_reason = "cleanup";
							break;
						};

						if let Err(e) = handle.record_activity().await {
							warn!(
								connection_id = %conn_key,
								error = ?e,
								"Failed to record inbound activity - closing"
							);
							instrument::record_error("activity_record_failed", "operation");
							end_reason = "error";
							break;
						}

						// Maintain message handling semantics
						if handle_websocket_message(
							msg,
							&state,
							transport.clone(),
							ws_tx.clone(),
							&conn_key
						)
							.await
								.is_err()
						{
							end_reason = "client_disconnect";
							break;
						}
					}

					Some(Err(e)) if is_peer_gone(&e) => {
						debug!(
							connection_id = %conn_key,
							error = %e,
							"Peer went away without a closing handshake"
						);
						end_reason = "client_disconnect";
						break;
					}

					Some(Err(e)) => {
						message_count += 1;
						error!(
							connection_id = %conn_key,
							message_number = message_count,
							error = %e,
							"WebSocket error"
						);
						instrument::record_error("stream_error", "operation");
						end_reason = "error";
						break;
					}

					None => {
						debug!(
							connection_id = %conn_key,
							"WebSocket stream ended"
						);
						end_reason = "client_disconnect";
						break;
					}
				}
			}
		}
	}

	(message_count, end_reason)
}

/// Whether a stream error only means the peer went away.
///
/// A client that drops TCP without a WebSocket close frame — a phone losing
/// signal, a laptop lid closing, the blackbox `ws_handshake` probe right after
/// its `101` — arrives here as an error, not as `None`: tungstenite reports
/// EOF on an open connection as `ResetWithoutClosingHandshake`, and a TCP
/// reset as an `Io` error. That is the client leaving, not a fault in this
/// server, so it ends the loop as `client_disconnect` instead of logging at
/// ERROR and counting `stream_error` — which, with the probe hanging up after
/// every handshake, fired every 15 seconds.
///
/// Downcasts to the `tungstenite::Error` axum wraps, so the `tungstenite`
/// dependency must match axum's; the real-socket test below goes through
/// axum's own upgrade, and fails if they ever drift apart.
fn is_peer_gone(error: &axum::Error) -> bool {
	use std::io::ErrorKind;
	use tungstenite::error::{Error, ProtocolError};

	let Some(source) = std::error::Error::source(error) else {
		return false;
	};
	match source.downcast_ref::<Error>() {
		Some(Error::ConnectionClosed | Error::AlreadyClosed | Error::Protocol(ProtocolError::ResetWithoutClosingHandshake)) => true,
		Some(Error::Io(io)) => matches!(
			io.kind(),
			ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe | ErrorKind::UnexpectedEof
		),
		_ => false,
	}
}

/// Handle a single WebSocket message based on its type
async fn handle_websocket_message(
	msg: Message,
	state: &WebSocketFsm,
	transport: NatsTransport<UnifiedEvent>,
	ws_tx: UnboundedSender<Event>,
	conn_key: &str,
) -> Result<(), ()> {
	match msg {
		Message::Text(text) => {
			instrument::record_message("text");
			// Process the message
			state.process_message(transport, ws_tx, conn_key, text).await;
			Ok(())
		}

		Message::Ping(_) => {
			instrument::record_message("ping");
			Ok(())
		}

		Message::Pong(_) => {
			instrument::record_message("pong");
			Ok(())
		}

		Message::Close(reason) => {
			instrument::record_message("close");
			let reason_str = reason
				.as_ref()
				.map(|f| format!("{}: {}", f.code, f.reason))
				.unwrap_or_else(|| "No reason provided".to_string());

			info!(
				connection_id = %conn_key,
				reason = %reason_str,
				"Client closed connection"
			);

			// Remove the connection (cleanup handled in remove_connection)
			let _ = state.remove_connection(conn_key, "WebSocket closed".to_string()).await;
			Err(())
		}

		Message::Binary(_) => {
			instrument::record_message("binary");
			Ok(())
		}
	}
}

#[cfg(test)]
mod tests {
	use super::is_peer_gone;
	use axum::{extract::ws::WebSocketUpgrade, routing::get, Router};
	use std::sync::{Arc, Mutex};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::{TcpListener, TcpStream},
		sync::oneshot,
		time::{timeout, Duration},
	};

	/// Upgrades one real connection through axum, lets `hang_up` end it from
	/// the client side without a close frame, and returns how the server's
	/// first `recv` classified that: `Some(true)` for "the peer went away",
	/// `Some(false)` for a real stream error, `None` if it wasn't an error.
	async fn server_view_of(hang_up: impl FnOnce(TcpStream)) -> Option<bool> {
		let (tx, rx) = oneshot::channel();
		let tx = Arc::new(Mutex::new(Some(tx)));
		let app = Router::new().route(
			"/ws",
			get(move |ws: WebSocketUpgrade| {
				let tx = tx.clone();
				async move {
					ws.on_upgrade(move |mut socket| async move {
						let verdict = match socket.recv().await {
							Some(Err(e)) => Some(is_peer_gone(&e)),
							_ => None,
						};
						let sender = tx.lock().unwrap().take();
						if let Some(sender) = sender {
							let _ = sender.send(verdict);
						}
					})
				}
			}),
		);
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

		let mut client = TcpStream::connect(addr).await.unwrap();
		client
			.write_all(
				b"GET /ws HTTP/1.1\r\nHost: t\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: x3JJHMbDL1EzLkh9GBhXDw==\r\n\r\n",
			)
			.await
			.unwrap();
		let mut buf = [0u8; 512];
		let n = client.read(&mut buf).await.unwrap();
		assert!(buf[..n].starts_with(b"HTTP/1.1 101"), "no upgrade: {:?}", String::from_utf8_lossy(&buf[..n]));

		hang_up(client);
		timeout(Duration::from_secs(5), rx).await.unwrap().unwrap()
	}

	/// What the blackbox `ws_handshake` probe does: read the `101`, then close
	/// the TCP connection without a close frame.
	#[tokio::test]
	async fn a_peer_that_closes_without_a_close_frame_is_a_disconnect() {
		assert_eq!(server_view_of(drop).await, Some(true));
	}

	/// A peer whose connection is reset outright (RST) rather than closed.
	#[tokio::test]
	async fn a_peer_that_resets_the_connection_is_a_disconnect() {
		let reset = |client: TcpStream| {
			// Deprecated because a non-zero linger blocks the thread on drop;
			// a zero linger doesn't wait for anything — it is the RST.
			#[allow(deprecated)]
			client.set_linger(Some(Duration::ZERO)).unwrap();
			drop(client);
		};
		assert_eq!(server_view_of(reset).await, Some(true));
	}
}

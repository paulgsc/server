use super::*;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::SplitSink;
use some_transport::{InMemTransportReceiver, Transport, TransportError};
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

type R = InMemTransportReceiver<Event>;
type T = Arc<InMemTransport<Event>>;

/// Pure broadcast result without side effects
#[derive(Debug)]
pub struct BroadcastResult {
	pub delivered: usize,
	pub failed: usize,
	pub duration: Duration,
}

impl BroadcastResult {
	fn success(count: usize, duration: Duration) -> Self {
		Self {
			delivered: count,
			failed: 0,
			duration,
		}
	}

	fn no_subscribers() -> Self {
		Self {
			delivered: 0,
			failed: 0,
			duration: Duration::default(),
		}
	}
}

impl WebSocketFsm {
	/// Spawns the event distribution task
	pub fn spawn_event_distribution_task(&self, cancel_token: CancellationToken) {
		let transport = self.transport.clone();
		let metrics = self.metrics.clone();
		let store = self.store.clone();

		tokio::spawn(async move {
			let mut receiver = transport.subscribe();

			loop {
				tokio::select! {
						_ = cancel_token.cancelled() => {
								info!("Event distribution task shutting down");
								break;
						}

						result = receiver.recv() => match result {
								Ok(event) => {
										if event.is_client_event() {
												let event_type_str = format!("{:?}", event.get_type().unwrap_or_default());
												let broadcast_result = timed_broadcast!(&event_type_str, {
														Self::broadcast_pure(&event, &store, transport.clone()).await
												});

												match broadcast_result {
														Ok(br) if br.delivered > 0 || br.failed > 0 => metrics.broadcast_attempt(br.failed == 0),
														Err(e) => {
																record_ws_error!("broadcast_failed", "main_channel", &e);
																metrics.broadcast_attempt(false);
																warn!("Broadcast failed: {}", e);
														}
														_ => {}
												}
										}
								}
								Err(TransportError::Closed) => {
										record_ws_error!("channel_closed", "main_receiver");
										break;
								}
								Err(TransportError::Overflowed(count)) => {
										record_ws_error!("channel_overflow", "main_receiver");
										warn!("Main receiver lagged behind by {} messages", count);
								}
								Err(e) => error!("Transport error in distribution loop: {}", e),
						}
				}
			}

			info!("Event distribution task exited");
		});
	}

	/// Pure broadcast function - no side effects
	pub async fn broadcast_pure(event: &Event, store: &Arc<ConnectionStore<EventType>>, transport: T) -> Result<BroadcastResult, TransportError> {
		if !event.is_client_event() {
			warn!("Skipping broadcast: event is not a client event: {:?}", event);
			return Ok(BroadcastResult::no_subscribers());
		}

		let start = Instant::now();
		let keys = store.keys();

		let event_key = match event.get_type() {
			Some(k) => k,
			None => {
				warn!("Could not determine event type for: {:?}", event);
				return Ok(BroadcastResult::no_subscribers());
			}
		};

		const BATCH_SIZE: usize = 256;
		let mut delivered = 0;
		let mut failed = 0;

		for chunk in keys.chunks(BATCH_SIZE) {
			let mut tasks = tokio::task::JoinSet::new();

			for key in chunk {
				if let Some(handle) = store.get(key) {
					let event_clone = event.clone();
					let transport_clone = transport.clone();
					let key_clone = key.clone();
					let event_key_clone = event_key.clone();

					tasks.spawn(async move {
						match handle.get_subscriptions().await {
							Ok(subs) if subs.contains(&event_key_clone) => match transport_clone.send(&key_clone, event_clone).await {
								Ok(_) => Ok(()),
								Err(_) => Err(()),
							},
							_ => Err(()),
						}
					});
				}
			}

			while let Some(result) = tasks.join_next().await {
				match result {
					Ok(Ok(())) => delivered += 1,
					Ok(Err(())) => failed += 1,
					Err(e) => {
						error!("Task panicked during broadcast: {}", e);
						failed += 1;
					}
				}
			}

			// Yield to prevent blocking
			tokio::task::yield_now().await;
		}

		let duration = start.elapsed();

		if delivered == 0 && failed == 0 {
			warn!("Broadcast succeeded but no subscribers received the event: {:?}, duration: {:?}", event, duration);
		}

		Ok(BroadcastResult::success(delivered, duration))
	}

	/// Public API method with full telemetry
	pub async fn broadcast_event(&self, event: &Event) -> ProcessResult {
		info!("Starting broadcast for event: {:?}", event);

		match Self::broadcast_pure(event, &self.store, self.transport.clone()).await {
			Ok(br) => {
				if br.delivered > 0 || br.failed > 0 {
					self.metrics.broadcast_attempt(br.failed == 0);
				}

				if br.failed > 0 {
					warn!(
						"Partial broadcast failure: delivered={}, failed={}, duration={:?}, event={:?}",
						br.delivered, br.failed, br.duration, event
					);
				}

				ProcessResult {
					delivered: br.delivered,
					failed: br.failed,
					duration: br.duration,
				}
			}
			Err(e) => {
				error!("Broadcast failed for event {:?}: {}", event, e);
				record_ws_error!("broadcast_failed", "main_channel", &e);
				self.metrics.broadcast_attempt(false);

				if let Some(event_type) = event.get_type() {
					self
						.emit_system_event(Event::BroadcastFailed {
							event_type,
							error: e.to_string(),
							affected_connections: 0,
						})
						.await;
				}

				ProcessResult {
					delivered: 0,
					failed: 1,
					duration: Duration::default(),
				}
			}
		}
	}

	pub(crate) async fn broadcast_client_count(&self) {
		let count = self.store.len();
		let event = Event::ClientCount { count };
		let _ = self.broadcast_event(&event).await;
		update_resource_usage!("active_connections", count as f64);
	}
}

// Spawns task to forward events from broadcast channel to WebSocket
pub(crate) fn spawn_event_forwarder(
	mut sender: SplitSink<WebSocket, Message>,
	mut event_receiver: R,
	state: WebSocketFsm,
	conn_key: String,
	cancel_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let mut message_count = 0u64;

		loop {
			tokio::select! {
					// Listen for cancellation signal
					_ = cancel_token.cancelled() => {
							info!(
									connection_id = %conn_key,
									messages_forwarded = message_count,
									"Event forwarder cancelled - shutting down"
							);
							break;
					}

					// Process incoming events
					result = event_receiver.recv() => {
							match result {
									Ok(event) => {
											if !event.is_client_event() {
													warn!("Event: {:?} is not a client event and is being ignored!", event);
													continue;
											}

											message_count += 1;

											if let Err(e) = forward_single_event(&mut sender, &event, &conn_key, message_count).await {
													record_ws_error!("forward_single_event_error", "forward", e);
													error!(
															connection_id = %conn_key,
															error = %e,
															"Error forwarding single event"
													);
													break; // Fatal: stop forwarding
											}

											if message_count % 100 == 0 {
													record_system_event!("forward_milestone", connection_id = conn_key, messages_forwarded = message_count);
													debug!(
															connection_id = %conn_key,
															messages_forwarded = message_count,
															"Forwarding milestone reached"
													);
											}
									}
									Err(TransportError::Overflowed(n)) => {
											// Client is lagging behind — warn, skip, but continue
											warn!(
													skipped = n,
													connection_id = %conn_key,
													"Event receiver lagged, skipped {} messages",
													n
											);
											record_ws_error!("receiver_overflow", "forward", n);
											continue;
									}
									Err(TransportError::Closed) => {
											// Fatal: channel closed → stop loop
											error!(
													connection_id = %conn_key,
													"Event channel closed, ending forwarder"
											);
											record_ws_error!("channel_closed", "forward");
											break;
									}
									Err(e) => {
											error!(
													connection_id = %conn_key,
													error = %e,
													"Transport error in forwarder"
											);
											break;
									}
							}
					}
			}
		}

		// Invariant: receiver gone → connection gone
		let _ = state.remove_connection(&conn_key, "Event forwarder ended".to_string()).await;
		record_system_event!("forward_ended", connection_id = conn_key, total_messages = message_count);
		debug!(
				connection_id = %conn_key,
				total_messages = message_count,
				"Event forwarding ended"
		);
	})
}

// Forwards a single event to the WebSocket client
async fn forward_single_event(sender: &mut SplitSink<WebSocket, Message>, event: &Event, conn_key: &str, message_count: u64) -> Result<(), String> {
	info!("Forwarding event #{} to client {}", message_count, conn_key);

	// Serialize event
	let json = timed_ws_operation!("forward", "serialize", { serde_json::to_string(event) }).map_err(|e| {
		let msg = format!("Failed to serialize event for client {}: {}", conn_key, e);
		record_ws_error!("serialization_failed", "forward", &msg);
		error!("{}", msg);
		msg
	})?;

	debug!("Serialized event for client {}: {}", conn_key, json);
	let msg = Message::Text(json);

	// Send message
	timed_ws_operation!("forward", "send", { sender.send(msg).await }).map_err(|e| {
		let msg = format!("Failed to forward event to client {} (msg #{}): {}", conn_key, message_count, e);
		record_ws_error!("forward_send_failed", "forward", &msg);
		error!("{}", msg);
		msg
	})?;

	info!("Successfully forwarded event #{} to client {}", message_count, conn_key);
	Ok(())
}

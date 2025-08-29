use super::*;
use async_broadcast::Receiver;
use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures::stream::SplitSink;
use obs_websocket::{ObsConfig, ObsRequestType, ObsWebSocketManager, PollingConfig, PollingFrequency, RetryConfig};
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub enum BroadcastOutcome {
	NoSubscribers,
	Completed { process_result: ProcessResult },
}

impl WebSocketFsm {
	/// Spawns the event distribution task - single responsibility
	pub fn spawn_event_distribution_task(&self) {
		let receiver = self.event_rcv.clone();
		let connections = self.connections.clone();
		let metrics = self.metrics.clone();

		tokio::spawn(async move {
			Self::event_distribution_loop(receiver, connections, metrics).await;
		});

		let _ = self.bridge_obs_events();
	}

	/// Event distribution loop - isolated logic
	async fn event_distribution_loop(mut receiver: broadcast::Receiver<Event>, connections: Arc<DashMap<String, Connection>>, metrics: Arc<ConnectionMetrics>) {
		loop {
			match receiver.recv().await {
				Ok(event) => {
					Self::handle_event_broadcast(event, connections.clone(), &metrics).await;
				}
				Err(e) => {
					if Self::handle_receiver_error(e) {
						break; // Exit loop on closed channel
					}
					// Continue on overflow
				}
			}
		}
	}

	/// Handles broadcasting a single event
	async fn handle_event_broadcast(event: Event, connections: Arc<DashMap<String, Connection>>, metrics: &Arc<ConnectionMetrics>) {
		let event_type = event.get_type();
		let event_type_str = format!("{:?}", event_type);

		let broadcast_outcome: Result<BroadcastOutcome, String> =
			timed_broadcast!(&event_type_str, { Ok(Self::broadcast_event_to_subscribers(event, &event_type, connections).await) });

		Self::handle_broadcast_outcome(broadcast_outcome, metrics);
	}

	/// Handles the result of a broadcast operation
	fn handle_broadcast_outcome(broadcast_outcome: Result<BroadcastOutcome, String>, metrics: &Arc<ConnectionMetrics>) {
		match broadcast_outcome {
			Ok(broadcast_outcome) => match broadcast_outcome {
				BroadcastOutcome::NoSubscribers => {
					// Nothing to do
				}
				BroadcastOutcome::Completed {
					process_result: ProcessResult { failed, .. },
				} => {
					metrics.broadcast_attempt(failed == 0);
				}
			},
			Err(_) => {
				record_ws_error!("channel_closed", "main_receiver");
			}
		}
	}

	/// Handles receiver errors, returns true if should exit loop
	fn handle_receiver_error(error: async_broadcast::RecvError) -> bool {
		match error {
			async_broadcast::RecvError::Closed => {
				record_ws_error!("channel_closed", "main_receiver", error);
				true // Exit the loop
			}
			async_broadcast::RecvError::Overflowed(count) => {
				record_ws_error!("channel_overflow", "main_receiver");
				warn!("Main receiver lagged behind by {} messages, continuing", count);
				false // Continue processing
			}
		}
	}

	/// Broadcasts an event to all subscribed and active connections
	pub(crate) async fn broadcast_event_to_subscribers(event: Event, event_type: &EventType, connections: Arc<DashMap<String, Connection>>) -> BroadcastOutcome {
		let start_time = Instant::now();
		let mut delivered = 0;
		let mut failed = 0;

		// Collect active connections that are subscribed to this event type
		let subscribed_connections: Vec<_> = connections
			.iter()
			.filter_map(|entry| {
				let conn = entry.value();
				if conn.is_active() && conn.is_subscribed_to(event_type) {
					Some((entry.key().clone(), conn.id.clone()))
				} else {
					None
				}
			})
			.collect();

		if subscribed_connections.is_empty() {
			return BroadcastOutcome::NoSubscribers;
		}

		// Send to each subscribed connection
		for (conn_key, connection_id) in subscribed_connections {
			if let Some(conn) = connections.get(&conn_key) {
				match conn.send_event(event.clone()).await {
					Ok(_) => delivered += 1,
					Err(e) => {
						failed += 1;
						record_ws_error!("send_failed", "broadcast", e);
						warn!("Failed to send event {:?} to client {}: {}", event_type, connection_id, e);
					}
				}
			}
		}
		let duration = start_time.elapsed();

		let process_result = ProcessResult { delivered, failed, duration };

		BroadcastOutcome::Completed { process_result }
	}

	pub async fn broadcast_event(&self, event: &Event) -> ProcessResult {
		let event_type_str = format!("{:?}", event.get_type());
		let receiver_count = self.sender.receiver_count();

		let result = timed_broadcast!(&event_type_str, {
			match self.sender.broadcast(event.clone()).await {
				Ok(_) => Ok(BroadcastOutcome::Completed {
					process_result: ProcessResult {
						delivered: receiver_count,
						failed: 0,
						duration: Duration::default(),
					},
				}),
				Err(e) => {
					record_ws_error!("broadcast_failed", "main_channel", e);
					self.metrics.broadcast_attempt(false);

					// Emit system event for monitoring
					record_system_event!("broadcast_failed", event_type = event.get_type(), error = e, affected_connections = receiver_count);

					Err(format!("Broadcast failed: {}", e))
				}
			}
		});

		match result {
			Ok(broadcast_outcome) => match broadcast_outcome {
				BroadcastOutcome::Completed { process_result } => process_result,
				BroadcastOutcome::NoSubscribers => ProcessResult {
					delivered: 0,
					failed: 0,
					duration: Duration::default(),
				},
			},
			Err(_) => ProcessResult {
				delivered: 0,
				failed: 1,
				duration: Duration::default(),
			},
		}
	}

	pub fn bridge_obs_events(&self) -> tokio::task::JoinHandle<()> {
		let metrics = self.metrics.clone();
		let conn_fan = self.connections.clone();

		tokio::spawn(async move {
			info!("Starting OBS event bridge with internal manager");

			let obs_config = ObsConfig::default();
			let obs_manager = ObsWebSocketManager::new(obs_config, RetryConfig::default());

			// Create a cancellation token to coordinate shutdown
			let cancel_token = CancellationToken::new();
			let cancel_token_clone = cancel_token.clone();

			// Spawn a task to handle shutdown signal
			let shutdown_task = tokio::spawn(async move {
				let _ = tokio::signal::ctrl_c().await;
				tracing::info!("Shutdown signal received");
				cancel_token_clone.cancel();
			});

			loop {
				tokio::select! {
					// Check for cancellation
					_ = cancel_token.cancelled() => {
						tracing::info!("Shutting down OBS bridge");
						let _ = obs_manager.disconnect().await;
						break;
					}

					// Main connection loop
					result = async {
						let requests = PollingConfig::default();
						let request_slice: Box<[(ObsRequestType, PollingFrequency)]> = requests.into();

						match obs_manager.connect(&request_slice).await {
							Ok(()) => {
								info!("Connected to OBS WebSocket");

								obs_manager.stream_events(|obs_event| {
									let metrics = metrics.clone();
									let conn_fan = conn_fan.clone();

									Box::pin(async move {
										let event = Event::ObsStatus { status: obs_event };
										let event_type = event.get_type();

										let broadcast_outcome = Self::broadcast_event_to_subscribers(
											event, &event_type
											,conn_fan
										).await;

										match broadcast_outcome {
											BroadcastOutcome::NoSubscribers => {
												warn!("no subscribers for obs!")
											}
											BroadcastOutcome::Completed {
												process_result: ProcessResult { delivered, failed, .. },
											} => {

												metrics.broadcast_attempt(failed == 0);
												debug!(
													"Event {:?} broadcast: {} delivered, {} failed",
													event_type, delivered, failed
												);
											}

										}
									})
								}).await
							}
							Err(e) => {
								error!("Failed to connect to OBS: {}", e);
								Err(e)
							}
						}
					} => {
						if let Err(e) = result {
							error!("OBS connection error: {}", e);
						}

						// Clean disconnect before retry
						let _ = obs_manager.disconnect().await;

						// Cancellable retry delay
						tokio::select! {
							_ = cancel_token.cancelled() => {
								tracing::info!("Shutdown during retry delay");
								break;
							}
							_ = tokio::time::sleep(Duration::from_secs(5)) => {
								// Continue to next retry
							}
						}

					}
				}
			}

			// Clean up shutdown task
			shutdown_task.abort();
			info!("OBS event bridge ended");
		})
	}

	pub(crate) async fn broadcast_client_count(&self) {
		let count = self.connections.len();
		let _ = self.sender.broadcast(Event::ClientCount { count }).await;
		update_resource_usage!("active_connections", count as f64);
	}
}

// Spawns task to forward events from broadcast channel to WebSocket
pub(crate) fn spawn_event_forwarder(
	mut sender: SplitSink<WebSocket, Message>,
	mut event_receiver: Receiver<Event>,
	state: WebSocketFsm,
	conn_key: String,
) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let mut message_count = 0u64;

		while let Ok(event) = event_receiver.recv().await {
			message_count += 1;

			if let Err(_) = forward_single_event(&mut sender, &event, &conn_key, message_count).await {
				break;
			}

			// Log periodic forwarding stats
			if message_count % 100 == 0 {
				record_system_event!("forward_milestone", connection_id = conn_key, messages_forwarded = message_count);
				debug!("Forwarded {} messages to client {}", message_count, conn_key);
			}
		}

		// Enforce invariant: receiver gone → connection gone
		let _ = state.remove_connection(&conn_key, "Event forwarder ended".to_string()).await;
		record_system_event!("forward_ended", connection_id = conn_key, total_messages = message_count);
		debug!("Event forwarding ended for client {} after {} messages", conn_key, message_count);
	})
}

// Forwards a single event to the WebSocket client
async fn forward_single_event(sender: &mut SplitSink<WebSocket, Message>, event: &Event, conn_key: &str, message_count: u64) -> Result<(), ()> {
	let result = timed_ws_operation!("forward", "serialize", { serde_json::to_string(event) });

	let msg = match result {
		Ok(json) => Message::Text(json),
		Err(e) => {
			record_ws_error!("serialization_failed", "forward", e);
			error!("Failed to serialize event for client {}: {}", conn_key, e);
			return Err(());
		}
	};

	let send_result = timed_ws_operation!("forward", "send", { sender.send(msg).await });

	if let Err(e) = send_result {
		record_ws_error!("forward_send_failed", "forward", e);
		error!("Failed to forward event to client {} (msg #{}): {}", conn_key, message_count, e);
		return Err(());
	}

	Ok(())
}

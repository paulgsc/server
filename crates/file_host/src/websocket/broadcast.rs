use super::*;
use async_broadcast::Receiver;
use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures::stream::SplitSink;
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

pub enum BroadcastOutcome {
	NoSubscribers,
	Completed { process_result: ProcessResult },
}

impl WebSocketFsm {
	/// Broadcasts an event to all subscribed and active connections
	pub(crate) async fn broadcast_event_to_subscribers(connections: &Arc<DashMap<String, Connection>>, event: Event, event_type: &EventType) -> BroadcastOutcome {
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
		for (client_key, connection_id) in subscribed_connections {
			if let Some(conn) = connections.get(&client_key) {
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

	pub fn bridge_obs_events(&self, obs_client: Arc<obs_websocket::ObsWebSocketWithBroadcast>) {
		let metrics = self.metrics.clone();
		let conn_fan = self.connections.clone();

		tokio::spawn(async move {
			let mut obs_receiver = obs_client.subscribe();
			info!("OBS event bridge started");

			loop {
				match tokio::time::timeout(Duration::from_secs(45), obs_receiver.recv()).await {
					Ok(Ok(obs_event)) => {
						let event = Event::ObsStatus { status: obs_event };
						let event_type = event.get_type();

						let broadcast_outcome = Self::broadcast_event_to_subscribers(&conn_fan, event, &event_type).await;
						match broadcast_outcome {
							BroadcastOutcome::NoSubscribers => continue,
							BroadcastOutcome::Completed {
								process_result: ProcessResult { delivered, failed, duration },
							} => {
								metrics.broadcast_attempt(failed == 0);
								debug!("Event {:?} broadcast: {} delivered, {} failed, took {:?}", event_type, delivered, failed, duration);

								if failed != 0 {
									tokio::time::sleep(Duration::from_millis(100)).await;
									continue;
								}
							}
						}
					}
					Ok(Err(e)) => match e {
						async_broadcast::RecvError::Closed => {
							error!("OBS receiver channel closed: {}", e);
							break;
						}
						async_broadcast::RecvError::Overflowed(count) => {
							warn!("OBS receiver lagged behind by {} messages, continuing", count);
							continue;
						}
					},
					Err(_) => {
						// Timeout - check connection status
						let is_connected = obs_client.is_connected().await;
						if !is_connected {
							warn!("OBS connection lost, bridge will retry when reconnected");
							tokio::time::sleep(Duration::from_secs(5)).await;
						}
						continue;
					}
				}
			}

			warn!("OBS event bridge ended");
		});
	}

	pub(crate) async fn broadcast_client_count(&self) {
		let count = self.connections.len();
		let _ = self.sender.broadcast(Event::ClientCount { count }).await;
		update_resource_usage!("active_connections", count as f64);
	}
}

// Spawns task to forward events from broadcast channel to WebSocket
pub(crate) fn spawn_event_forwarder(mut sender: SplitSink<WebSocket, Message>, mut event_receiver: Receiver<Event>, client_key: String) -> tokio::task::JoinHandle<()> {
	tokio::spawn(async move {
		let mut message_count = 0u64;

		while let Ok(event) = event_receiver.recv().await {
			message_count += 1;

			if let Err(_) = forward_single_event(&mut sender, &event, &client_key, message_count).await {
				break;
			}

			// Log periodic forwarding stats
			if message_count % 100 == 0 {
				record_system_event!("forward_milestone", connection_id = client_key, messages_forwarded = message_count);
				debug!("Forwarded {} messages to client {}", message_count, client_key);
			}
		}

		record_system_event!("forward_ended", connection_id = client_key, total_messages = message_count);
		debug!("Event forwarding ended for client {} after {} messages", client_key, message_count);
	})
}

// Forwards a single event to the WebSocket client
async fn forward_single_event(sender: &mut SplitSink<WebSocket, Message>, event: &Event, client_key: &str, message_count: u64) -> Result<(), ()> {
	let result = timed_ws_operation!("forward", "serialize", { serde_json::to_string(event) });

	let msg = match result {
		Ok(json) => Message::Text(json),
		Err(e) => {
			record_ws_error!("serialization_failed", "forward", e);
			error!("Failed to serialize event for client {}: {}", client_key, e);
			return Err(());
		}
	};

	let send_result = timed_ws_operation!("forward", "send", { sender.send(msg).await });

	if let Err(e) = send_result {
		record_ws_error!("forward_send_failed", "forward", e);
		error!("Failed to forward event to client {} (msg #{}): {}", client_key, message_count, e);
		return Err(());
	}

	Ok(())
}

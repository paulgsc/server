use crate::metrics::*;
use crate::transport::ConnectionReceivers;
use crate::WebSocketFsm;
use tokio::time::{Duration, Instant};
use tracing::{error, info, warn};
use ws_connection::ConnectionId;

pub(crate) mod handlers;
pub(crate) mod types;

pub(crate) use handlers::process_incoming_messages;
use types::{ClientMessage, ProcessResult};

impl WebSocketFsm {
	/// Process a text message from a client
	pub async fn process_message(&self, conn_key: &str, raw_message: String, receivers: &ConnectionReceivers) {
		let start = Instant::now();

		// Get connection info
		let connection_id = match self.store.get(conn_key) {
			Some(handle) => handle.connection.id.clone(),
			None => {
				record_ws_error!("connection_not_found", "message_processing");
				error!("Cannot process message for unknown client: {}", conn_key);
				return;
			}
		};

		let message_id = MessageId::new();

		// Parse the message
		let client_message = match serde_json::from_str::<ClientMessage>(&raw_message) {
			Ok(msg) => msg,
			Err(e) => {
				let duration = start.elapsed();
				record_ws_error!("parse_error", "message", e);
				self.metrics.message_processed(false);
				self.send_error_to_client(conn_key, &format!("Invalid JSON: {}", e)).await;

				// Emit system event
				self
					.record_message_processed(message_id, connection_id, duration, ProcessResult::failure(1, duration))
					.await;
				return;
			}
		};

		// Handle the message based on its type
		let result = match client_message {
			// Heartbeat response
			ClientMessage::Pong => {
				self.handle_pong(conn_key).await;
				ProcessResult::success(1, start.elapsed())
			}

			// Subscription management
			ClientMessage::Subscribe { event_types } => {
				self.handle_subscribe(conn_key, event_types, receivers).await;
				ProcessResult::success(1, start.elapsed())
			}

			ClientMessage::Unsubscribe { event_types } => {
				self.handle_unsubscribe(conn_key, event_types, receivers).await;
				ProcessResult::success(1, start.elapsed())
			}

			// Unknown/unsupported message type
			ClientMessage::Unknown { type_name } => {
				warn!(
					connection_id = %connection_id,
					message_type = %type_name,
					"Received unsupported message type"
				);
				self.send_error_to_client(conn_key, &format!("Unsupported message type: {}", type_name)).await;
				ProcessResult::failure(1, start.elapsed())
			}
		};

		let duration = start.elapsed();
		let success = result.failed == 0;

		// Update metrics
		self.metrics.message_processed(success);

		// Emit system event for observability
		self.record_message_processed(message_id, connection_id, duration, result).await;
	}

	/// Handle pong message (heartbeat response)
	async fn handle_pong(&self, conn_key: &str) {
		if let Err(e) = self.update_client_ping(conn_key).await {
			record_ws_error!("ping_update_failed", "connection", e);
			warn!("Failed to update ping for {}: {}", conn_key, e);
		}
	}

	/// Handle subscribe request - add new event type subscriptions
	async fn handle_subscribe(&self, conn_key: &str, event_types: Vec<EventType>, receivers: &ConnectionReceivers) {
		let start = Instant::now();

		// Update NATS subscriptions and actor state
		if let Err(e) = self.handle_subscription_update(conn_key, event_types.clone(), vec![], receivers).await {
			record_ws_error!("subscription_failed", "subscribe", e);
			error!(
				connection_id = %conn_key,
				error = %e,
				"Failed to subscribe to event types"
			);
			self.send_error_to_client(conn_key, &format!("Subscription failed: {}", e)).await;
			return;
		}

		let duration = start.elapsed();

		info!(
			connection_id = %conn_key,
			event_types = ?event_types,
			duration_ms = duration.as_millis(),
			"Successfully subscribed to event types"
		);

		// Send confirmation to client
		if let Err(e) = self.send_subscription_ack(conn_key, event_types).await {
			warn!("Failed to send subscription acknowledgment: {}", e);
		}
	}

	/// Handle unsubscribe request - remove event type subscriptions
	async fn handle_unsubscribe(&self, conn_key: &str, event_types: Vec<EventType>, receivers: &ConnectionReceivers) {
		let start = Instant::now();

		// Update NATS subscriptions and actor state
		if let Err(e) = self.handle_subscription_update(conn_key, vec![], event_types.clone(), receivers).await {
			record_ws_error!("unsubscription_failed", "unsubscribe", e);
			error!(
				connection_id = %conn_key,
				error = %e,
				"Failed to unsubscribe from event types"
			);
			self.send_error_to_client(conn_key, &format!("Unsubscription failed: {}", e)).await;
			return;
		}

		let duration = start.elapsed();

		info!(
			connection_id = %conn_key,
			event_types = ?event_types,
			duration_ms = duration.as_millis(),
			"Successfully unsubscribed from event types"
		);

		// Send confirmation to client
		if let Err(e) = self.send_unsubscription_ack(conn_key, event_types).await {
			warn!("Failed to send unsubscription acknowledgment: {}", e);
		}
	}

	/// Send subscription acknowledgment to client
	async fn send_subscription_ack(&self, conn_key: &str, event_types: Vec<EventType>) -> Result<(), String> {
		let ack = Event::System(SystemEvent {
			event_type: "subscription_ack".to_string(),
			payload: serde_json::to_vec(&serde_json::json!({
				"subscribed": event_types.iter().map(|t| format!("{:?}", t)).collect::<Vec<_>>(),
			}))
			.unwrap_or_default(),
		});

		self.send_event_to_connection(conn_key, ack).await
	}

	/// Send unsubscription acknowledgment to client
	async fn send_unsubscription_ack(&self, conn_key: &str, event_types: Vec<EventType>) -> Result<(), String> {
		let ack = Event::System(SystemEvent {
			event_type: "unsubscription_ack".to_string(),
			payload: serde_json::to_vec(&serde_json::json!({
				"unsubscribed": event_types.iter().map(|t| format!("{:?}", t)).collect::<Vec<_>>(),
			}))
			.unwrap_or_default(),
		});

		self.send_event_to_connection(conn_key, ack).await
	}

	/// Update client ping timestamp via heartbeat manager
	async fn update_client_ping(&self, client_key: &str) -> Result<(), String> {
		self.heartbeat_manager.record_ping(client_key).await;
		Ok(())
	}

	/// Send error message to a specific client
	async fn send_error_to_client(&self, client_key: &str, error: &str) {
		let error_event = Event::Error { message: error.to_string() };

		if let Err(e) = self.send_event_to_connection(client_key, error_event).await {
			record_ws_error!("error_send_failed", "connection", e);
			warn!("Failed to send error to client {}: {}", client_key, e);
		}
	}

	/// Record a message processing event
	async fn record_message_processed(&self, message_id: MessageId, connection_id: ConnectionId, duration: Duration, result: ProcessResult) {
		let system_event = Event::System(SystemEvent {
			event_type: "message_processed".to_string(),
			payload: serde_json::to_vec(&serde_json::json!({
				"message_id": message_id.to_string(),
				"connection_id": connection_id.as_string(),
				"duration_ms": duration.as_millis(),
				"delivered": result.delivered,
				"failed": result.failed,
			}))
			.unwrap_or_default(),
		});

		// Broadcast to system event subscribers (monitoring/observability)
		let _ = self.broadcast_event(system_event).await;
	}
}

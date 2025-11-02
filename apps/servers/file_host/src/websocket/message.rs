use crate::WebSocketFsm;
use some_transport::{NatsTransport, UnboundedSenderExt};
use tokio::{sync::mpsc::UnboundedSender, time::Instant};
use tracing::{error, info};
use ws_events::events::{Event, EventType, SystemEvent, UnifiedEvent};

pub(crate) mod handlers;

pub(crate) use handlers::process_incoming_messages;

impl WebSocketFsm {
	/// Process a text message from a client
	pub async fn process_message(&self, transport: NatsTransport<UnifiedEvent>, ws_tx: UnboundedSender<Event>, conn_key: &str, raw_message: String) {
		// Parse the message
		let client_message = match serde_json::from_str::<Event>(&raw_message) {
			Ok(msg) => msg,
			Err(e) => {
				self.send_error_to_client(ws_tx, &format!("Invalid JSON: {}", e));
				return;
			}
		};

		// Handle the message based on its type
		match client_message {
			// Subscription management
			Event::Subscribe { event_types } => {
				self.handle_subscribe(ws_tx, conn_key, event_types).await;
			}

			Event::Unsubscribe { event_types } => {
				self.handle_unsubscribe(ws_tx, conn_key, event_types).await;
			}

			Event::ObsCmd { .. } => {
				let _ = self.broadcast_event(transport, client_message).await;
			}
			_ => {}
		};
	}

	/// Handle subscribe request - add new event type subscriptions
	async fn handle_subscribe(&self, ws_tx: UnboundedSender<Event>, conn_key: &str, event_types: Vec<EventType>) {
		let start = Instant::now();

		// Update actor state
		if let Err(e) = self.handle_subscription_update(conn_key, event_types.clone(), vec![]).await {
			error!(
				connection_id = %conn_key,
				error = %e,
				"Failed to subscribe to event types"
			);
			self.send_error_to_client(ws_tx, &format!("Subscription failed: {}", e));
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
		self.send_subscription_ack(ws_tx, conn_key, event_types).await
	}

	/// Handle unsubscribe request - remove event type subscriptions
	async fn handle_unsubscribe(&self, ws_tx: UnboundedSender<Event>, conn_key: &str, event_types: Vec<EventType>) {
		let start = Instant::now();

		// Update NATS subscriptions and actor state
		if let Err(e) = self.handle_subscription_update(conn_key, vec![], event_types.clone()).await {
			error!(
				connection_id = %conn_key,
				error = %e,
				"Failed to unsubscribe from event types"
			);
			self.send_error_to_client(ws_tx, &format!("Unsubscription failed: {}", e));
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
		self.send_unsubscription_ack(ws_tx, conn_key, event_types).await
	}

	/// Send subscription acknowledgment to client
	async fn send_subscription_ack(&self, ws_tx: UnboundedSender<Event>, conn_key: &str, event_types: Vec<EventType>) {
		let ack = Event::System(SystemEvent::ConnectionStateChanged {
			connection_id: conn_key.to_owned(),
			from: "subscribed".to_owned(),
			to: "subscribed".to_owned(),
			metadata: serde_json::json!({
				"subscribed": event_types.iter().map(|t| format!("{:?}", t)).collect::<Vec<_>>(),
			}),
		});

		let context = "subscription_ack";
		ws_tx.send_graceful(ack, context);
	}

	/// Send unsubscription acknowledgment to client
	async fn send_unsubscription_ack(&self, ws_tx: UnboundedSender<Event>, conn_key: &str, event_types: Vec<EventType>) {
		let ack = Event::System(SystemEvent::ConnectionStateChanged {
			connection_id: conn_key.to_owned(),
			from: "subscribed".to_owned(),
			to: "subscribed".to_owned(),
			metadata: serde_json::json!({
				"unsubscribed": event_types.iter().map(|t| format!("{:?}", t)).collect::<Vec<_>>(),
			}),
		});

		let context = "subscription_ack";
		ws_tx.send_graceful(ack, context);
	}

	/// Send error message to a specific client
	fn send_error_to_client(&self, ws_tx: UnboundedSender<Event>, error: &str) {
		let error_event = Event::Error { message: error.to_string() };

		let context = "client_err_msg";
		ws_tx.send_graceful(error_event, context);
	}
}

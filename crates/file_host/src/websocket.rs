use crate::*;
use crate::{utils::generate_uuid, UtteranceMetadata};
use async_broadcast::{broadcast, Receiver, Sender};
use axum::{
	extract::{
		ws::{Message, WebSocket, WebSocketUpgrade},
		State,
	},
	response::IntoResponse,
	routing::get,
	Router,
};
use dashmap::DashMap;
use futures::{sink::SinkExt, stream::StreamExt};
use obs_websocket::ObsEvent;
use serde::{Deserialize, Serialize};
use std::{
	collections::HashSet,
	fmt,
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc,
	},
};
use tokio::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

// Connection ID type for type safety
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionId([u8; 32]);

impl ConnectionId {
	pub fn new() -> Self {
		Self(generate_uuid())
	}

	pub fn from_buffer(buffer: [u8; 32]) -> Self {
		Self(buffer)
	}

	pub fn as_string(&self) -> String {
		// Convert to hex string for reliable string representation
		hex::encode(&self.0)
	}

	pub fn as_bytes(&self) -> &[u8; 32] {
		&self.0
	}
}

impl fmt::Display for ConnectionId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.as_string())
	}
}

// Message correlation ID for tracing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageId([u8; 32]);

impl MessageId {
	pub fn new() -> Self {
		Self(generate_uuid())
	}

	pub fn as_string(&self) -> String {
		hex::encode(&self.0)
	}
}

impl fmt::Display for MessageId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.as_string())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
	ObsStatus,
	ClientCount,
	Ping,
	Pong,
	Error,
	TabMetaData,
	Utterance,
}

#[derive(Clone, Serialize, Debug, Deserialize)]
pub struct NowPlaying {
	title: String,
	channel: String,
	video_id: String,
	current_time: u32,
	duration: u32,
	thumbnail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Event {
	ObsStatus { status: ObsEvent },
	TabMetaData { data: NowPlaying },
	ClientCount { count: usize },
	Ping,
	Pong,
	Error { message: String },
	Subscribe { event_types: Vec<EventType> },
	Unsubscribe { event_types: Vec<EventType> },
	Utterance { text: String, metadata: UtteranceMetadata },
}

impl Event {
	pub fn get_type(&self) -> EventType {
		match self {
			Self::Ping => EventType::Ping,
			Self::Pong => EventType::Pong,
			Self::Error { .. } => EventType::Error,
			Self::Subscribe { .. } => EventType::Ping, // These are control messages
			Self::Unsubscribe { .. } => EventType::Ping,
			Self::ClientCount { .. } => EventType::ClientCount,
			Self::ObsStatus { .. } => EventType::ObsStatus,
			Self::TabMetaData { .. } => EventType::TabMetaData,
			Self::Utterance { .. } => EventType::Utterance,
		}
	}
}

impl From<NowPlaying> for Event {
	fn from(data: NowPlaying) -> Self {
		Event::TabMetaData { data }
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UtterancePrompt {
	pub text: String,
	pub metadata: UtteranceMetadata,
}

impl From<UtterancePrompt> for Event {
	fn from(UtterancePrompt { text, metadata }: UtterancePrompt) -> Self {
		Event::Utterance { text, metadata }
	}
}

#[derive(Debug, Clone)]
pub enum ConnectionState {
	Active { last_ping: Instant },
	Stale { last_ping: Instant, reason: String },
	Disconnected { reason: String, disconnected_at: Instant },
}

impl fmt::Display for ConnectionState {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Active { last_ping } => {
				write!(f, "Active (last_ping: {:?})", last_ping)
			}
			Self::Stale { last_ping, reason } => {
				write!(f, "Stale (last_ping: {:?}, reason: {})", last_ping, reason)
			}
			Self::Disconnected { reason, disconnected_at } => {
				write!(f, "Disconnected (reason: {}, at: {:?})", reason, disconnected_at)
			}
		}
	}
}

// System event for observability and debugging
#[derive(Debug, Clone)]
pub enum SystemEvent {
	ConnectionStateChanged {
		connection_id: ConnectionId,
		from: ConnectionState,
		to: ConnectionState,
	},
	MessageProcessed {
		message_id: MessageId,
		connection_id: ConnectionId,
		duration: Duration,
		result: ProcessResult,
	},
	BroadcastFailed {
		event_type: EventType,
		error: String,
		affected_connections: usize,
	},
	ConnectionCleanup {
		connection_id: ConnectionId,
		reason: String,
		resources_freed: bool,
	},
}

// Connection metrics for monitoring
#[derive(Debug, Default)]
pub struct ConnectionMetrics {
	pub total_created: AtomicU64,
	pub total_removed: AtomicU64,
	pub current_active: AtomicU64,
	pub current_stale: AtomicU64,
	pub messages_processed: AtomicU64,
	pub messages_failed: AtomicU64,
	pub broadcast_succeeded: AtomicU64,
	pub broadcast_failed: AtomicU64,
}

impl ConnectionMetrics {
	pub fn connection_created(&self) {
		self.total_created.fetch_add(1, Ordering::Relaxed);
		self.current_active.fetch_add(1, Ordering::Relaxed);
	}

	pub fn connection_removed(&self, was_active: bool) {
		self.total_removed.fetch_add(1, Ordering::Relaxed);
		if was_active {
			self.current_active.fetch_sub(1, Ordering::Relaxed);
		} else {
			self.current_stale.fetch_sub(1, Ordering::Relaxed);
		}
	}

	pub fn connection_marked_stale(&self) {
		self.current_active.fetch_sub(1, Ordering::Relaxed);
		self.current_stale.fetch_add(1, Ordering::Relaxed);
	}

	pub fn message_processed(&self, success: bool) {
		if success {
			self.messages_processed.fetch_add(1, Ordering::Relaxed);
		} else {
			self.messages_failed.fetch_add(1, Ordering::Relaxed);
		}
	}

	pub fn broadcast_attempt(&self, success: bool) {
		if success {
			self.broadcast_succeeded.fetch_add(1, Ordering::Relaxed);
		} else {
			self.broadcast_failed.fetch_add(1, Ordering::Relaxed);
		}
	}

	pub fn get_snapshot(&self) -> ConnectionMetricsSnapshot {
		ConnectionMetricsSnapshot {
			total_created: self.total_created.load(Ordering::Relaxed),
			total_removed: self.total_removed.load(Ordering::Relaxed),
			current_active: self.current_active.load(Ordering::Relaxed),
			current_stale: self.current_stale.load(Ordering::Relaxed),
			messages_processed: self.messages_processed.load(Ordering::Relaxed),
			messages_failed: self.messages_failed.load(Ordering::Relaxed),
			broadcast_succeeded: self.broadcast_succeeded.load(Ordering::Relaxed),
			broadcast_failed: self.broadcast_failed.load(Ordering::Relaxed),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionMetricsSnapshot {
	pub total_created: u64,
	pub total_removed: u64,
	pub current_active: u64,
	pub current_stale: u64,
	pub messages_processed: u64,
	pub messages_failed: u64,
	pub broadcast_succeeded: u64,
	pub broadcast_failed: u64,
}

// Connection FSM container with proper resource management
#[derive(Debug)]
pub struct Connection {
	pub id: ConnectionId,
	pub established_at: Instant,
	pub state: ConnectionState,
	pub sender: Sender<Event>,
	pub subscriptions: HashSet<EventType>,
	pub message_count: u64,
	pub last_message_at: Instant,
}

impl Connection {
	pub fn new() -> (Self, Receiver<Event>) {
		let (mut sender, receiver) = broadcast::<Event>(1);
		sender.set_await_active(false); // Prevent blocking on slow clients
		sender.set_overflow(true);

		let mut subscriptions = HashSet::new();
		subscriptions.insert(EventType::Ping);
		subscriptions.insert(EventType::Pong);
		subscriptions.insert(EventType::Error);
		subscriptions.insert(EventType::ClientCount); // Always subscribe to client count

		let connection = Self {
			id: ConnectionId::new(),
			established_at: Instant::now(),
			state: ConnectionState::Active { last_ping: Instant::now() },
			sender,
			subscriptions,
			message_count: 0,
			last_message_at: Instant::now(),
		};

		(connection, receiver)
	}

	pub fn update_ping(&mut self) -> Result<ConnectionState, String> {
		let now = Instant::now();
		let old_state = self.state.clone();
		match &mut self.state {
			ConnectionState::Active { last_ping } => {
				*last_ping = now;
				self.last_message_at = now;
				Ok(old_state)
			}
			_ => Err("Cannot update ping on non-active connection".to_string()),
		}
	}

	pub fn subscribe(&mut self, event_types: Vec<EventType>) -> usize {
		let initial_count = self.subscriptions.len();
		for t in event_types {
			self.subscriptions.insert(t);
		}
		self.subscriptions.len() - initial_count
	}

	pub fn unsubscribe(&mut self, event_types: Vec<EventType>) -> usize {
		let initial_count = self.subscriptions.len();
		for t in event_types {
			self.subscriptions.remove(&t);
		}
		initial_count - self.subscriptions.len()
	}

	pub fn mark_stale(&mut self, reason: String) -> Result<ConnectionState, String> {
		let old_state = self.state.clone();
		match &self.state {
			ConnectionState::Active { last_ping } => {
				self.state = ConnectionState::Stale { last_ping: *last_ping, reason };
				Ok(old_state)
			}
			_ => Err("Can only mark active connections as stale".to_string()),
		}
	}

	pub fn disconnect(&mut self, reason: String) -> Result<ConnectionState, String> {
		let old_state = self.state.clone();
		self.state = ConnectionState::Disconnected {
			reason,
			disconnected_at: Instant::now(),
		};
		Ok(old_state)
	}

	pub fn is_active(&self) -> bool {
		matches!(self.state, ConnectionState::Active { .. })
	}

	pub fn is_stale(&self, timeout: Duration) -> bool {
		match &self.state {
			ConnectionState::Active { last_ping } => Instant::now().duration_since(*last_ping) > timeout,
			ConnectionState::Stale { .. } => true,
			_ => false,
		}
	}

	pub fn is_subscribed_to(&self, event_type: &EventType) -> bool {
		self.subscriptions.contains(event_type)
	}

	pub async fn send_event(&self, event: Event) -> Result<(), String> {
		if !self.is_active() {
			return Err(format!("Cannot send to non-active connection (state: {})", self.state));
		}

		match self.sender.broadcast(event).await {
			Ok(_) => Ok(()),
			Err(e) => Err(format!("Failed to send event to client channel: {}", e)),
		}
	}

	pub fn increment_message_count(&mut self) {
		self.message_count += 1;
		self.last_message_at = Instant::now();
	}
}

// Enhanced message FSM with correlation tracking
#[derive(Debug)]
pub enum MessageState {
	Received { raw: String },
	Parsed { event: Event },
	Validated { event: Event },
	Processed { event: Event, result: ProcessResult },
	Failed { error: String },
}

#[derive(Debug, Clone)]
pub struct ProcessResult {
	pub delivered: usize,
	pub failed: usize,
	pub duration: Duration,
}

#[derive(Debug)]
pub struct EventMessage {
	pub id: MessageId,
	pub connection_id: ConnectionId,
	pub timestamp: Instant,
	pub state: MessageState,
}

impl EventMessage {
	pub fn new(connection_id: ConnectionId, raw: String) -> Self {
		Self {
			id: MessageId::new(),
			connection_id,
			timestamp: Instant::now(),
			state: MessageState::Received { raw },
		}
	}

	pub fn parse(&mut self) -> Result<(), String> {
		match &self.state {
			MessageState::Received { raw } => match serde_json::from_str::<Event>(raw) {
				Ok(event) => {
					self.state = MessageState::Parsed { event };
					Ok(())
				}
				Err(e) => {
					let error = format!("Parse error: {}", e);
					self.state = MessageState::Failed { error: error.clone() };
					Err(error)
				}
			},
			_ => Err("Can only parse received messages".to_string()),
		}
	}

	pub fn validate(&mut self) -> Result<(), String> {
		match &self.state {
			MessageState::Parsed { event } => match event {
				Event::Error { message } if message.is_empty() => {
					let error = "Error event cannot have empty message".to_string();
					self.state = MessageState::Failed { error: error.clone() };
					Err(error)
				}
				_ => {
					self.state = MessageState::Validated { event: event.clone() };
					Ok(())
				}
			},
			_ => Err("Can only validate parsed messages".to_string()),
		}
	}

	pub fn mark_processed(&mut self, result: ProcessResult) {
		if let MessageState::Validated { event } = &self.state {
			self.state = MessageState::Processed { event: event.clone(), result };
		}
	}

	pub fn get_event(&self) -> Option<&Event> {
		match &self.state {
			MessageState::Parsed { event } | MessageState::Validated { event } | MessageState::Processed { event, .. } => Some(event),
			_ => None,
		}
	}

	pub fn duration_since_creation(&self) -> Duration {
		Instant::now().duration_since(self.timestamp)
	}
}

pub enum BroadcastOutcome {
	NoSubscribers,
	Completed { delivered: u32, failed: u32 },
}

// Enhanced WebSocket FSM with comprehensive observability
#[derive(Clone)]
pub struct WebSocketFsm {
	connections: Arc<DashMap<String, Connection>>,
	sender: Sender<Event>,
	metrics: Arc<ConnectionMetrics>,
	system_events: Sender<SystemEvent>,
}

impl WebSocketFsm {
	pub fn new() -> Self {
		let (mut sender, receiver) = broadcast::<Event>(1000); // Larger buffer for main channel
		sender.set_await_active(false);
		sender.set_overflow(true);

		let (system_sender, _system_receiver) = broadcast::<SystemEvent>(500);

		let connections = Arc::new(DashMap::<String, Connection>::new());
		let metrics = Arc::new(ConnectionMetrics::default());

		// Event distribution task
		let conn_fan = connections.clone();
		let metrics_clone = metrics.clone();
		// let system_events_clone = system_sender.clone();

		tokio::spawn(async move {
			let mut receiver = receiver;

			loop {
				match receiver.recv().await {
					Ok(event) => {
						let event_type = event.get_type();
						let event_type_str = format!("{:?}", event_type);

						let broadcast_outcome = timed_broadcast!(&event_type_str, { Self::broadcast_event_to_subscribers(&conn_fan, event, &event_type).await });

						match broadcast_outcome {
							BroadcastOutcome::NoSubscribers => continue,
							BroadcastOutcome::Completed { failed, .. } => {
								metrics_clone.broadcast_attempt(failed == 0);
							}
						}
					}
					Err(e) => match e {
						async_broadcast::RecvError::Closed => {
							record_ws_error!("channel_closed", "main_receiver", e);
							break;
						}
						async_broadcast::RecvError::Overflowed(count) => {
							record_ws_error!("channel_overflow", "main_receiver");
							warn!("Main receiver lagged behind by {} messages, continuing", count);
							continue;
						}
					},
				}
			}
		});

		record_system_event!("fsm_initialized");
		update_resource_usage!("active_connections", 0.0);

		Self {
			connections,
			sender,
			metrics,
			system_events: system_sender,
		}
	}

	/// Broadcasts an event to all subscribed and active connections
	/// Returns (delivered_count, failed_count)
	async fn broadcast_event_to_subscribers(connections: &Arc<DashMap<String, Connection>>, event: Event, event_type: &EventType) -> BroadcastOutcome {
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

		BroadcastOutcome::Completed { delivered, failed }
	}

	pub fn router(self) -> Router {
		Router::new().route("/ws", get(websocket_handler)).with_state(self)
	}

	// Enhanced message processing with full traceability
	pub async fn process_message(&self, client_key: &str, raw_message: String) {
		// Get connection ID for correlation
		let connection_id = if let Some(conn) = self.connections.get(client_key) {
			conn.id.clone()
		} else {
			error!("Cannot process message for unknown client: {}", client_key);
			return;
		};

		let mut message = EventMessage::new(connection_id.clone(), raw_message);
		let start_time = Instant::now();

		// Update message count
		if let Some(mut conn) = self.connections.get_mut(client_key) {
			conn.increment_message_count();
		}

		// Parse
		if let Err(e) = message.parse() {
			error!("Message {} parse failed for client {}: {}", message.id, connection_id, e);
			self.metrics.message_processed(false);
			self.send_error_to_client(client_key, &e).await;
			return;
		}

		// Handle control messages immediately
		if let Some(event) = message.get_event() {
			match event {
				Event::Pong => {
					if let Err(e) = self.update_client_ping(client_key).await {
						error!("Failed to update ping for client {}: {}", connection_id, e);
					}
					self.metrics.message_processed(true);
					return;
				}
				Event::Subscribe { event_types } => {
					self.handle_subscription(client_key, event_types.clone(), true).await;
					self.metrics.message_processed(true);
					return;
				}
				Event::Unsubscribe { event_types } => {
					self.handle_subscription(client_key, event_types.clone(), false).await;
					self.metrics.message_processed(true);
					return;
				}
				_ => {}
			}
		}

		// Validate
		if let Err(e) = message.validate() {
			error!("Message {} validation failed for client {}: {}", message.id, connection_id, e);
			self.metrics.message_processed(false);
			self.send_error_to_client(client_key, &e).await;
			return;
		}

		// Process (broadcast)
		if let Some(event) = message.get_event() {
			let result = self.broadcast_event(event).await;
			let duration = start_time.elapsed();

			let process_result = ProcessResult {
				delivered: result.delivered,
				failed: result.failed,
				duration,
			};

			message.mark_processed(process_result.clone());
			self.metrics.message_processed(true);

			// Emit system event for monitoring
			let _ = self
				.system_events
				.broadcast(SystemEvent::MessageProcessed {
					message_id: message.id.clone(),
					connection_id,
					duration,
					result: process_result,
				})
				.await;
		}
	}

	async fn handle_subscription(&self, client_key: &str, event_types: Vec<EventType>, subscribe: bool) {
		if let Some(mut conn) = self.connections.get_mut(client_key) {
			let changed_count = if subscribe {
				conn.subscribe(event_types.clone())
			} else {
				conn.unsubscribe(event_types.clone())
			};

			debug!(
				"Client {} {} {} event types: {:?}",
				conn.id,
				if subscribe { "subscribed to" } else { "unsubscribed from" },
				changed_count,
				event_types
			);
		}
	}

	pub async fn broadcast_event(&self, event: &Event) -> ProcessResult {
		let start_time = Instant::now();
		let receiver_count = self.sender.receiver_count();

		match self.sender.broadcast(event.clone()).await {
			Ok(_) => {
				let duration = start_time.elapsed();
				ProcessResult {
					delivered: receiver_count,
					failed: 0,
					duration,
				}
			}
			Err(e) => {
				error!("Failed to broadcast event: {}", e);
				self.metrics.broadcast_attempt(false);

				// Emit system event for monitoring
				let _ = self
					.system_events
					.broadcast(SystemEvent::BroadcastFailed {
						event_type: event.get_type(),
						error: e.to_string(),
						affected_connections: receiver_count,
					})
					.await;

				ProcessResult {
					delivered: 0,
					failed: 1,
					duration: start_time.elapsed(),
				}
			}
		}
	}

	// Enhanced connection management with proper resource tracking
	pub async fn add_connection(&self) -> Result<(String, Receiver<Event>), String> {
		let (connection, receiver) = Connection::new();
		let client_key = connection.id.as_string();
		let connection_id = connection.id.clone();

		self.connections.insert(client_key.clone(), connection);
		self.metrics.connection_created();

		info!("Connection {} added successfully", connection_id);
		Ok((client_key, receiver))
	}

	pub async fn remove_connection(&self, client_key: &str, reason: String) -> Result<(), String> {
		if let Some((_, mut connection)) = self.connections.remove(client_key) {
			let connection_id = connection.id.clone();
			let was_active = connection.is_active();

			// Transition to disconnected state
			let old_state = connection.disconnect(reason.clone())?;
			self.metrics.connection_removed(was_active);

			// Emit system events
			let _ = self
				.system_events
				.broadcast(SystemEvent::ConnectionStateChanged {
					connection_id: connection_id.clone(),
					from: old_state,
					to: connection.state.clone(),
				})
				.await;

			let _ = self
				.system_events
				.broadcast(SystemEvent::ConnectionCleanup {
					connection_id: connection_id.clone(),
					reason: reason.clone(),
					resources_freed: true,
				})
				.await;

			info!("Connection {} removed: {}", connection_id, reason);

			// Update and broadcast client count
			self.broadcast_client_count().await;
		}
		Ok(())
	}

	async fn broadcast_client_count(&self) {
		let count = self.connections.len();
		let _ = self.sender.broadcast(Event::ClientCount { count }).await;
		debug!("Broadcasted client count: {}", count);
	}

	// Enhanced timeout monitor with invariant checking
	pub fn start_timeout_monitor(&self, timeout: Duration) {
		let connections = self.connections.clone();
		let metrics = self.metrics.clone();
		let system_events = self.system_events.clone();
		let sender = self.sender.clone();

		tokio::spawn(async move {
			let mut interval = tokio::time::interval(Duration::from_secs(30));

			loop {
				interval.tick().await;

				// Check invariants
				let total_connections = connections.len();
				let metrics_snapshot = metrics.get_snapshot();
				let expected_active = metrics_snapshot.total_created - metrics_snapshot.total_removed;

				if total_connections as u64 != expected_active {
					warn!("Connection count invariant violated: actual={}, expected={}", total_connections, expected_active);
				}

				// Find stale connections (collect keys to avoid holding iterator during modifications)
				let stale_connection_keys: Vec<String> = connections
					.iter()
					.filter_map(|entry| {
						let conn = entry.value();
						if conn.is_stale(timeout) {
							Some(entry.key().clone())
						} else {
							None
						}
					})
					.collect();

				// Process stale connections
				let mut cleaned_up = 0;
				for client_key in stale_connection_keys {
					if let Some(mut entry) = connections.get_mut(&client_key) {
						let conn = entry.value_mut();
						let connection_id = conn.id.clone();

						if let Ok(old_state) = conn.mark_stale("Timeout".to_string()) {
							metrics.connection_marked_stale();

							// Emit state change event
							let _ = system_events
								.broadcast(SystemEvent::ConnectionStateChanged {
									connection_id: connection_id.clone(),
									from: old_state,
									to: conn.state.clone(),
								})
								.await;

							warn!("Connection {} marked as stale due to timeout", connection_id);
						}
					}

					// Remove stale connection
					if let Some((_, mut conn)) = connections.remove(&client_key) {
						let connection_id = conn.id.clone();
						let _ = conn.disconnect("Timeout cleanup".to_string());
						cleaned_up += 1;

						let _ = system_events
							.broadcast(SystemEvent::ConnectionCleanup {
								connection_id,
								reason: "Timeout cleanup".to_string(),
								resources_freed: true,
							})
							.await;
					}
				}

				if cleaned_up > 0 {
					info!("Cleaned up {} stale connections", cleaned_up);
					let count = connections.len();
					let _ = sender.broadcast(Event::ClientCount { count }).await;
				}

				// Log periodic health metrics
				debug!(
					"Connection health: total={}, active={}, stale={}, msgs_processed={}, msgs_failed={}",
					total_connections, metrics_snapshot.current_active, metrics_snapshot.current_stale, metrics_snapshot.messages_processed, metrics_snapshot.messages_failed
				);
			}
		});
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
						let start_time = Instant::now();
						let event = Event::ObsStatus { status: obs_event };
						let event_type = event.get_type();

						let broadcast_outcome = Self::broadcast_event_to_subscribers(&conn_fan, event, &event_type).await;
						match broadcast_outcome {
							BroadcastOutcome::NoSubscribers => continue,
							BroadcastOutcome::Completed { delivered, failed } => {
								let duration = start_time.elapsed();
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

	async fn send_error_to_client(&self, client_key: &str, error: &str) {
		if let Some(connection) = self.connections.get(client_key) {
			let error_event = Event::Error { message: error.to_string() };
			if let Err(e) = connection.send_event(error_event).await {
				warn!("Failed to send error to client {}: {}", connection.id, e);
			}
		}
	}

	async fn update_client_ping(&self, client_key: &str) -> Result<(), String> {
		if let Some(mut connection) = self.connections.get_mut(client_key) {
			let connection_id = connection.id.clone();
			match connection.update_ping() {
				Ok(old_state) => {
					// Emit state change event if there was a transition
					if !matches!(old_state, ConnectionState::Active { .. }) {
						let _ = self
							.system_events
							.broadcast(SystemEvent::ConnectionStateChanged {
								connection_id,
								from: old_state,
								to: connection.state.clone(),
							})
							.await;
					}
					Ok(())
				}
				Err(e) => Err(format!("Failed to update ping for {}: {}", connection_id, e)),
			}
		} else {
			Err(format!("Client {} not found", client_key))
		}
	}

	pub async fn get_client_count(&self) -> usize {
		self.connections.len()
	}

	pub fn get_metrics(&self) -> ConnectionMetricsSnapshot {
		self.metrics.get_snapshot()
	}

	pub fn subscribe_to_system_events(&self) -> Receiver<SystemEvent> {
		self.system_events.new_receiver()
	}

	// Health check endpoint data
	pub async fn get_health_status(&self) -> HealthStatus {
		let health_result: Result<HealthStatus, ()> = health_check!("health_status", {
			let metrics = self.get_metrics();
			let connection_states = self.get_connection_state_distribution().await;

			// Check system invariants
			check_invariant!(!self.sender.is_closed(), "sender_state", "Main sender channel is closed");

			check_invariant!(
				self.sender.receiver_count() > 0 || self.connections.is_empty(),
				"receiver_count",
				"No receivers but connections exist",
				expected: "receivers > 0 or connections == 0",
				actual: format!("receivers: {}, connections: {}", self.sender.receiver_count(), self.connections.len())
			);

			Ok(HealthStatus {
				total_connections: self.connections.len(),
				metrics,
				connection_states,
				sender_receiver_count: self.sender.receiver_count(),
				sender_is_closed: self.sender.is_closed(),
			})
		});

		match health_result {
			Ok(status) => status,
			Err(_) => {
				record_ws_error!("health_check_failed", "health_status");
				// Return degraded status
				HealthStatus {
					total_connections: self.connections.len(),
					metrics: self.get_metrics(),
					connection_states: ConnectionStateDistribution {
						active: 0,
						stale: 0,
						disconnected: 0,
					},
					sender_receiver_count: 0,
					sender_is_closed: true,
				}
			}
		}
	}

	async fn get_connection_state_distribution(&self) -> ConnectionStateDistribution {
		let mut active = 0;
		let mut stale = 0;
		let mut disconnected = 0;

		for entry in self.connections.iter() {
			match entry.value().state {
				ConnectionState::Active { .. } => active += 1,
				ConnectionState::Stale { .. } => stale += 1,
				ConnectionState::Disconnected { .. } => disconnected += 1,
			}
		}

		ConnectionStateDistribution { active, stale, disconnected }
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
	pub total_connections: usize,
	pub metrics: ConnectionMetricsSnapshot,
	pub connection_states: ConnectionStateDistribution,
	pub sender_receiver_count: usize,
	pub sender_is_closed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStateDistribution {
	pub active: usize,
	pub stale: usize,
	pub disconnected: usize,
}

async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<WebSocketFsm>) -> impl IntoResponse {
	ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: WebSocketFsm) {
	let (mut sender, mut receiver) = socket.split();

	// Create connection through FSM with proper resource management
	let (client_key, mut event_receiver) = match state.add_connection().await {
		Ok((key, rx)) => (key, rx),
		Err(e) => {
			error!("Failed to add connection: {}", e);
			return;
		}
	};

	info!("WebSocket connection established: {}", client_key);

	// Send initial ping
	let ping_event = Event::Ping;
	if let Ok(msg) = serde_json::to_string(&ping_event) {
		if let Err(e) = sender.send(Message::Text(msg)).await {
			error!("Failed to send initial ping to {}: {}", client_key, e);
		}
	}

	// Broadcast updated client count
	state.broadcast_client_count().await;

	// Forward events from broadcast channel to websocket
	let forward_task = {
		let client_key_clone = client_key.clone();
		tokio::spawn(async move {
			let mut message_count = 0u64;

			while let Ok(event) = event_receiver.recv().await {
				message_count += 1;

				let msg = match serde_json::to_string(&event) {
					Ok(json) => Message::Text(json),
					Err(e) => {
						error!("Failed to serialize event for client {}: {}", client_key_clone, e);
						continue;
					}
				};

				if let Err(e) = sender.send(msg).await {
					error!("Failed to forward event to client {} (msg #{}): {}", client_key_clone, message_count, e);
					break;
				}

				// Log periodic forwarding stats
				if message_count % 100 == 0 {
					debug!("Forwarded {} messages to client {}", message_count, client_key_clone);
				}
			}

			debug!("Event forwarding ended for client {} after {} messages", client_key_clone, message_count);
		})
	};

	// Process incoming messages with enhanced error handling
	let mut message_count = 0u64;
	while let Some(result) = receiver.next().await {
		message_count += 1;

		match result {
			Ok(msg) => match msg {
				Message::Text(text) => {
					debug!("Received message #{} from {}: {} chars", message_count, client_key, text.len());
					state.process_message(&client_key, text).await;
				}
				Message::Ping(_) => {
					debug!("Received WebSocket ping from {}", client_key);
					if let Err(e) = state.update_client_ping(&client_key).await {
						warn!("Failed to update ping for {}: {}", client_key, e);
					}
				}
				Message::Pong(_) => {
					debug!("Received WebSocket pong from {}", client_key);
					if let Err(e) = state.update_client_ping(&client_key).await {
						warn!("Failed to update pong for {}: {}", client_key, e);
					}
				}
				Message::Close(reason) => {
					info!("Client {} closed connection: {:?}", client_key, reason);
					break;
				}
				_ => {
					debug!("Ignored message type from {}", client_key);
				}
			},
			Err(e) => {
				error!("WebSocket error for {} (msg #{}): {}", client_key, message_count, e);
				break;
			}
		}
	}

	// Clean up through FSM with comprehensive logging
	info!("Cleaning up connection {} after {} messages", client_key, message_count);

	if let Err(e) = state.remove_connection(&client_key, "Connection closed".to_string()).await {
		error!("Failed to remove connection {}: {}", client_key, e);
	}

	forward_task.abort();
	info!("Connection {} cleanup completed", client_key);
}

pub async fn init_websocket() -> WebSocketFsm {
	record_system_event!("websocket_init_started");
	let state = WebSocketFsm::new();

	// Start FSM processes with instrumentation
	state.start_timeout_monitor(Duration::from_secs(120));

	// Start system event monitoring for debugging with enhanced instrumentation
	let system_events = state.subscribe_to_system_events();
	tokio::spawn(async move {
		let mut events = system_events;
		record_system_event!("system_event_monitor_started");

		while let Ok(event) = events.recv().await {
			match event {
				SystemEvent::ConnectionStateChanged { connection_id, from, to } => {
					record_system_event!("connection_state_changed", connection_id = connection_id, from_state = from, to_state = to);
					info!("Connection {} state: {} -> {}", connection_id, from, to);
				}
				SystemEvent::MessageProcessed {
					message_id,
					connection_id,
					duration,
					result,
				} => {
					record_system_event!(
						"message_processed",
						message_id = message_id,
						connection_id = connection_id,
						duration_ms = duration.as_millis(),
						delivered = result.delivered,
						failed = result.failed
					);
					debug!(
						"Message {} from {} processed in {:?}: {} delivered, {} failed",
						message_id, connection_id, duration, result.delivered, result.failed
					);
				}
				SystemEvent::BroadcastFailed {
					event_type,
					error,
					affected_connections,
				} => {
					record_system_event!("broadcast_failed", event_type = event_type, error = error, affected_connections = affected_connections);
					error!("Broadcast failed for {:?} affecting {} connections: {}", event_type, affected_connections, error);
				}
				SystemEvent::ConnectionCleanup {
					connection_id,
					reason,
					resources_freed,
				} => {
					record_system_event!("connection_cleanup", connection_id = connection_id, reason = reason, resources_freed = resources_freed);
					info!("Connection {} cleaned up (reason: {}, resources freed: {})", connection_id, reason, resources_freed);
				}
			}
		}

		record_system_event!("system_event_monitor_ended");
	});

	record_system_event!("websocket_init_completed");
	info!("Enhanced FSM WebSocket system initialized with full observability and instrumentation");
	state
}

// Re-export for compatibility
pub use WebSocketFsm as WebSocketState;

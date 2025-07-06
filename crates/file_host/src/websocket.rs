use crate::utils::generate_uuid;
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
	sync::Arc,
	time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
	ObsStatus,
	ClientCount,
	Ping,
	Pong,
	Error,
	TabMetaData,
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
}

impl Event {
	pub fn get_type(&self) -> EventType {
		match self {
			Self::Ping => EventType::Ping,
			Self::Pong => EventType::Pong,
			Self::Error { .. } => EventType::Error,
			Self::Subscribe { .. } => EventType::Ping,
			Self::Unsubscribe { .. } => EventType::Ping,
			Self::ClientCount { .. } => EventType::ClientCount,
			Self::ObsStatus { .. } => EventType::ObsStatus,
			Self::TabMetaData { .. } => EventType::TabMetaData,
		}
	}
}

impl From<NowPlaying> for Event {
	fn from(data: NowPlaying) -> Self {
		Event::TabMetaData { data }
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
			Self::Active { last_ping, .. } => {
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

// Connection FSM container
#[derive(Debug)]
pub struct Connection {
	pub id: [u8; 32],
	pub established_at: Instant,
	pub state: ConnectionState,
	pub sender: Sender<Event>,
	pub subscriptions: HashSet<EventType>,
}

// FSM Transitions - Strictly Enforced
impl Connection {
	pub fn new(sender: Sender<Event>) -> Self {
		let mut subscriptions = HashSet::new();
		subscriptions.insert(EventType::Ping);
		subscriptions.insert(EventType::Pong);
		subscriptions.insert(EventType::Error);

		Self {
			id: generate_uuid(),
			established_at: Instant::now(),
			state: ConnectionState::Active { last_ping: Instant::now() },
			sender,
			subscriptions,
		}
	}

	pub fn update_ping(&mut self) -> Result<(), String> {
		match &mut self.state {
			ConnectionState::Active { last_ping, .. } => {
				*last_ping = Instant::now();
				Ok(())
			}
			_ => Err("Cannot update ping on non-active connection".to_string()),
		}
	}

	pub fn subscribe(&mut self, event_types: Vec<EventType>) {
		for t in event_types {
			self.subscriptions.insert(t);
		}
	}

	pub fn unsubscribe(&mut self, event_types: Vec<EventType>) {
		for t in event_types {
			self.subscriptions.remove(&t);
		}
	}

	pub fn mark_stale(&mut self, reason: String) -> Result<(), String> {
		match &self.state {
			ConnectionState::Active { last_ping, .. } => {
				self.state = ConnectionState::Stale { last_ping: *last_ping, reason };
				Ok(())
			}
			_ => Err("Can only mark active connections as stale".to_string()),
		}
	}

	pub fn disconnect(&mut self, reason: String) -> Result<(), String> {
		self.state = ConnectionState::Disconnected {
			reason,
			disconnected_at: Instant::now(),
		};
		Ok(())
	}

	pub fn is_active(&self) -> bool {
		matches!(self.state, ConnectionState::Active { .. })
	}

	pub fn is_stale(&self, timeout: Duration) -> bool {
		match &self.state {
			ConnectionState::Active { last_ping, .. } => Instant::now().duration_since(*last_ping) > timeout,
			_ => false,
		}
	}

	pub fn is_subscribed_to(&self, event_type: &EventType) -> bool {
		self.subscriptions.contains(event_type)
	}

	pub async fn send_event(&self, event: Event) -> Result<(), String> {
		if !self.is_active() {
			return Err("Cannot send to non-active connection".to_string());
		}

		self.sender.broadcast(event).await.map_err(|_| "Failed to send event to client channel".to_string())?;
		Ok(())
	}
}

// Message FSM container
#[derive(Debug)]
pub enum MessageState {
	Received { raw: String },
	Parsed { event: Event },
	Validated { event: Event },
	Processed { event: Event, result: ProcessResult },
	Failed { error: String },
}

#[derive(Debug)]
pub struct ProcessResult {
	pub delivered: usize,
	pub failed: usize,
}

#[derive(Debug)]
pub struct EventMessage {
	pub id: [u8; 32],
	pub timestamp: Instant,
	pub state: MessageState,
}

// FSM Transitions with Result handling
impl EventMessage {
	pub fn new(raw: String) -> Self {
		let message = Self {
			id: generate_uuid(),
			timestamp: Instant::now(),
			state: MessageState::Received { raw },
		};
		message
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
}

// TODO: FIX BELOW
// *********** Code above is working vvvvvvvvvvvvvvvv Code below is now working for reason

#[derive(Clone)]
pub struct WebSocketFsm {
	// Active connections - only Connected state stored here
	connections: Arc<DashMap<String, Connection>>,

	// Event broadcaster
	sender: Sender<Event>,

	// Client count for metrics
	client_count: Arc<RwLock<usize>>,
}

impl WebSocketFsm {
	pub fn new() -> Self {
		let (mut sender, mut receiver) = broadcast::<Event>(100);

		sender.set_await_active(true);
		sender.set_overflow(true);

		let connections = Arc::new(DashMap::<String, Connection>::new());
		let conn_fan = connections.clone();

		tokio::spawn(async move {
			loop {
				match receiver.recv().await {
					Ok(event) => {
						let event_type = event.get_type();

						for entry in conn_fan.iter() {
							let conn = entry.value();

							if conn.is_active() && conn.is_subscribed_to(&event_type) {
								match conn.send_event(event.clone()).await {
									Ok(_) => {}
									Err(e) => {
										warn!("Failed to send event to client {:?}: {}", conn.id, e);
									}
								}
							}
						}
					}
					Err(e) => match e {
						async_broadcast::RecvError::Closed => {
							error!("OBS reciever channel closed: {}", e);
							break;
						}
						async_broadcast::RecvError::Overflowed(count) => {
							warn!("OBS receiver lagged behind by {} messages, continuing", count);
							continue;
						}
					},
				}
			}
		});

		Self {
			connections,
			sender,
			client_count: Arc::new(RwLock::new(0)),
		}
	}

	pub fn router(self) -> Router {
		Router::new().route("/ws", get(websocket_handler)).with_state(self)
	}

	// Process incoming message through FSM pipeline with proper error handling
	pub async fn process_message(&self, client_id: &str, raw_message: String) {
		let mut message = EventMessage::new(raw_message);
		let message_id = message.id;

		// Parse
		match message.parse() {
			Ok(p) => p,
			Err(failed) => {
				error!("Message {:?} parse failed for client {}: {}", message_id, client_id, failed);
				self.send_error_to_client(client_id, &failed).await;
				return;
			}
		};

		if let Some(event) = message.get_event() {
			match event {
				Event::Pong => {
					if let Err(e) = self.update_client_ping(client_id).await {
						warn!("Failed to update ping for client {}: {}", client_id, e);
					}
					return;
				}
				Event::Subscribe { event_types } => {
					self.handle_subscription(client_id, event_types.clone(), true).await;
					return;
				}
				Event::Unsubscribe { event_types } => {
					self.handle_subscription(client_id, event_types.clone(), false).await;
					return;
				}
				_ => {}
			}
		}

		// Validate
		match message.validate() {
			Ok(v) => v,
			Err(failed) => {
				error!("Message {:?} validation failed for client {}: {}", message_id, client_id, failed);
				self.send_error_to_client(client_id, &failed).await;
				return;
			}
		};

		// Process (broadcast)
		if let Some(event) = message.get_event() {
			let result = self.broadcast_event(event).await;
			message.mark_processed(result);
		}
	}

	async fn handle_subscription(&self, client_id: &str, event_types: Vec<EventType>, subscribe: bool) {
		if let Some(mut conn) = self.connections.get_mut(client_id) {
			if subscribe {
				conn.subscribe(event_types.clone());
			} else {
				conn.unsubscribe(event_types.clone());
			}
		}
	}

	pub async fn broadcast_event(&self, event: &Event) -> ProcessResult {
		let receiver_count = self.sender.receiver_count();

		match self.sender.broadcast(event.clone()).await {
			Ok(_) => ProcessResult {
				delivered: receiver_count,
				failed: 0,
			},
			Err(e) => {
				error!("Failed to broadcast event: {}", e);
				ProcessResult { delivered: 0, failed: 1 }
			}
		}
	}

	// Add new connection through FSM
	pub async fn add_connection(&self, client_id: &str) -> Result<Receiver<Event>, String> {
		let (mut sender, receiver) = broadcast::<Event>(100); // This is for individual client messages, not events

		sender.set_await_active(true);
		sender.set_overflow(true);

		let connection = Connection::new(sender);

		self.connections.insert(client_id.to_string(), connection);
		self.update_client_count_and_broadcast(1).await;

		// Return a receiver for the main event broadcast
		Ok(receiver)
	}

	// Remove connection through FSM
	pub async fn remove_connection(&self, client_id: &str, reason: String) -> Result<(), String> {
		if let Some((_, mut connection)) = self.connections.remove(client_id) {
			connection.disconnect(reason)?;
			drop(connection);
			self.update_client_count_and_broadcast(-1).await;
		}
		Ok(())
	}

	async fn update_client_count_and_broadcast(&self, delta: isize) {
		let mut count = self.client_count.write().await;
		if delta > 0 {
			*count += delta as usize;
		} else {
			*count = count.saturating_sub((-delta) as usize)
		}
		let new_count = *count;
		drop(count);

		let _ = self.sender.broadcast(Event::ClientCount { count: new_count }).await;
	}

	// FSM-aware timeout checker
	pub fn start_timeout_monitor(&self, timeout: Duration) {
		let connections = self.connections.clone();
		let client_count = self.client_count.clone();
		let sender = self.sender.clone();

		tokio::spawn(async move {
			let mut interval = tokio::time::interval(Duration::from_secs(30));

			loop {
				interval.tick().await;
				let mut stale_clients = Vec::new();

				// Find stale connections
				for mut entry in connections.iter_mut() {
					let conn = entry.value_mut();
					if conn.is_stale(timeout) {
						if let Err(e) = conn.mark_stale("Timeout".to_string()) {
							error!("Failed to mark client {:?} as stale: {}", conn.id, e);
						} else {
							stale_clients.push(conn.id.clone());
						}
					}
				}

				// Remove stale connections through FSM
				if !stale_clients.is_empty() {
					let mut removed_count = 0;

					for client_id in stale_clients {
						if let Some((_, mut conn)) = connections.remove(std::str::from_utf8(&client_id).unwrap()) {
							if conn.disconnect("Timeout".to_string()).is_ok() {
								removed_count += 1;
								warn!("Client {:?} timed out and disconnected", client_id);
							}
						}
					}

					if removed_count > 0 {
						let mut count = client_count.write().await;
						*count = count.saturating_sub(removed_count);
						let new_count = *count;
						drop(count);

						let _ = sender.broadcast(Event::ClientCount { count: new_count }).await;
					}
				}
			}
		});
	}

	pub fn bridge_obs_events(&self, obs_client: Arc<obs_websocket::ObsWebSocketWithBroadcast>) {
		let sender = self.sender.clone();

		tokio::spawn(async move {
			let mut obs_receiver = obs_client.subscribe();
			info!("OBS event bridge started");

			loop {
				match tokio::time::timeout(Duration::from_secs(45), obs_receiver.recv()).await {
					Ok(Ok(obs_event)) => {
						let event = Event::ObsStatus { status: obs_event };

						// Always try to broadcast - the persistent receiver should keep the channel open
						match sender.broadcast(event.clone()).await {
							Ok(_) => {
								// Successfully broadcasted
							}
							Err(e) => {
								error!("Event broadcast channel closed unexpectedly: {}", e);
								error!("Receiver count: {}", sender.receiver_count());
								error!("Is closed: {}", sender.is_closed());

								// This shouldn't happen with persistent receiver, but let's handle it
								tokio::time::sleep(Duration::from_millis(100)).await;
								continue;
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
						// Timeout waiting for OBS event
						let is_connected = obs_client.is_connected().await;
						if !is_connected {
							warn!("OBS connection lost, bridge will retry when reconnected");
							// Sleep a bit longer when disconnected to avoid busy waiting
							tokio::time::sleep(Duration::from_secs(5)).await;
						}
						continue;
					}
				}
			}

			warn!("OBS event bridge ended");
		});
	}

	async fn send_error_to_client(&self, client_id: &str, error: &str) {
		if let Some(connection) = self.connections.get(client_id) {
			let error_event = Event::Error { message: error.to_string() };
			let _ = connection.send_event(error_event).await;
		}
	}

	async fn update_client_ping(&self, client_id: &str) -> Result<(), String> {
		if let Some(mut connection) = self.connections.get_mut(client_id) {
			connection.update_ping()
		} else {
			Err(format!("Client {:?} not found", client_id))
		}
	}

	pub async fn get_client_count(&self) -> usize {
		*self.client_count.read().await
	}
}

async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<WebSocketFsm>) -> impl IntoResponse {
	ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: WebSocketFsm) {
	let (mut sender, mut receiver) = socket.split();

	// Create client channel
	let client_id = generate_uuid();

	// Add connection through FSM
	let mut event_receiver = match state.add_connection(std::str::from_utf8(&client_id).unwrap().into()).await {
		Ok(rx) => rx,
		Err(e) => {
			error!("Failed to add connection: {}", e);
			return;
		}
	};

	// Send initial ping
	let ping_event = Event::Ping;
	let msg = serde_json::to_string(&ping_event).unwrap();
	if let Err(e) = sender.send(Message::Text(msg)).await {
		error!("Failed to send initial ping: {}", e);
	}

	// Forward events from broadcast channel to websocket
	let forward_task = {
		let client_id = client_id.clone();
		tokio::spawn(async move {
			while let Ok(event) = event_receiver.recv().await {
				let msg = match serde_json::to_string(&event) {
					Ok(json) => Message::Text(json),
					Err(e) => {
						error!("Failed to serialize event for client {:?}: {}", client_id, e);
						continue;
					}
				};

				if let Err(e) = sender.send(msg).await {
					error!("Failed to forward event to client {:?}: {}", client_id, e);
					break;
				}
			}
		})
	};

	// Process incoming messages
	while let Some(result) = receiver.next().await {
		match result {
			Ok(msg) => match msg {
				Message::Text(text) => {
					state.process_message(std::str::from_utf8(&client_id).unwrap().into(), text).await;
				}
				Message::Ping(_) => {
					if let Err(e) = state.update_client_ping(std::str::from_utf8(&client_id).unwrap().into()).await {
						warn!("Failed to update ping for {:?}: {}", client_id, e);
					}
				}
				Message::Pong(_) => {
					if let Err(e) = state.update_client_ping(std::str::from_utf8(&client_id).unwrap().into()).await {
						warn!("Failed to update pong for {:?}: {}", client_id, e);
					}
				}
				Message::Close(reason) => {
					warn!("Client {:?} closed: {:?}", client_id, reason);
					break;
				}
				_ => {} // Ignore other message types
			},
			Err(e) => {
				error!("WebSocket error for {:?}: {}", client_id, e);
				break;
			}
		}
	}

	// Clean up through FSM
	if let Err(e) = state
		.remove_connection(std::str::from_utf8(&client_id).unwrap().into(), "Connection closed".to_string())
		.await
	{
		error!("Failed to remove connection {:?}: {}", client_id, e);
	}
	forward_task.abort();
}

pub async fn init_websocket() -> WebSocketFsm {
	let state = WebSocketFsm::new();

	// Start FSM processes
	state.start_timeout_monitor(Duration::from_secs(120));

	info!("FSM WebSocket system initialized");
	state
}

// Re-export for compatibility
pub use WebSocketFsm as WebSocketState;

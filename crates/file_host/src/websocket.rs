use crate::utils::generate_uuid;
use axum::{
	extract::{
		ws::{Message, WebSocket, WebSocketUpgrade},
		State,
	},
	response::IntoResponse,
	routing::get,
	Router,
};
use futures::{sink::SinkExt, stream::StreamExt};
use obs_websocket::ObsEvent;
use serde::{Deserialize, Serialize};
use std::{
	collections::HashMap,
	fmt,
	sync::Arc,
	time::{Duration, Instant},
};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
	#[serde(rename = "obsStatus")]
	ObsStatus { status: ObsEvent },
	#[serde(rename = "clientCount")]
	ClientCount { count: usize },
	#[serde(rename = "ping")]
	Ping,
	#[serde(rename = "pong")]
	Pong,
	#[serde(rename = "error")]
	Error { message: String },
}

pub struct Connecting;
pub struct Connected {
	pub sender: mpsc::Sender<Message>,
	pub last_ping: Instant,
}
pub struct Stale {
	pub sender: mpsc::Sender<Message>,
	pub last_ping: Instant,
	pub reason: String,
}
pub struct Disconnected {
	pub reason: String,
}

impl fmt::Display for Connecting {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "State: Connecting")
	}
}

impl fmt::Display for Connected {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "State: Connected (last ping: {:?})", self.last_ping)
	}
}

impl fmt::Display for Stale {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "State: Stale (last ping: {:?}, reason: {})", self.last_ping, self.reason)
	}
}

impl fmt::Display for Disconnected {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "State: Disconnected (reason: {})", self.reason)
	}
}

// Connection FSM container
#[derive(Debug)]
pub struct Connection<S> {
	pub id: [u8; 32],
	pub established_at: Instant,
	pub state: S,
}

// FSM Transitions - Strictly Enforced
impl Connection<Connecting> {
	pub fn new() -> Self {
		let connection = Self {
			id: generate_uuid(),
			established_at: Instant::now(),
			state: Connecting,
		};
		connection
	}

	pub fn establish(self, sender: mpsc::Sender<Message>) -> Result<Connection<Connected>, String> {
		info!("Connection {:?} transitioning from Connecting to {}", self.id, self.state);
		Ok(Connection {
			id: self.id,
			established_at: self.established_at,
			state: Connected {
				sender,
				last_ping: Instant::now(),
			},
		})
	}
}

impl Connection<Connected> {
	pub fn update_ping(&mut self) -> Result<(), String> {
		self.state.last_ping = Instant::now();
		debug!("Connection {:?} ping updated", self.id);
		Ok(())
	}

	pub fn mark_stale(self, reason: String) -> Result<Connection<Stale>, String> {
		warn!("{:?} transitioning from {} to Stale: {}", self.id, self.state, reason);
		Ok(Connection {
			id: self.id,
			established_at: self.established_at,
			state: Stale {
				sender: self.state.sender,
				last_ping: self.state.last_ping,
				reason,
			},
		})
	}

	pub fn disconnect(self, reason: String) -> Result<Connection<Disconnected>, String> {
		info!("Connection {:?} transitioning from Connected to Disconnected: {}", self.id, reason);
		Ok(Connection {
			id: self.id,
			established_at: self.established_at,
			state: Disconnected { reason },
		})
	}

	pub async fn send_event(&self, event: &Event) -> Result<(), String> {
		let msg = serde_json::to_string(event).map_err(|e| format!("Serialize error: {}", e))?;

		self.state.sender.send(Message::Text(msg)).await.map_err(|e| format!("Send error: {}", e))
	}
}

impl Connection<Stale> {
	pub fn disconnect(self) -> Result<Connection<Disconnected>, String> {
		info!("Connection {:?} transitioning from Stale to Disconnected", self.id);
		Ok(Connection {
			id: self.id,
			established_at: self.established_at,
			state: Disconnected {
				reason: format!("Stale connection: {}", self.state.reason),
			},
		})
	}
}

// Message states
pub struct Received {
	pub raw: String,
}
pub struct Parsed {
	pub event: Event,
}
pub struct Validated {
	pub event: Event,
}
pub struct Queued {
	pub event: Event,
	pub queued_at: Instant,
}
pub struct Broadcasting {
	pub event: Event,
	pub started_at: Instant,
}
pub struct Delivered {
	pub event: Event,
	pub delivered_count: usize,
	pub failed_count: usize,
	pub completed_at: Instant,
}
pub struct DeliveryFailed {
	pub event: Event,
	pub error: String,
	pub failed_at: Instant,
}
pub struct ParseFailed {
	pub error: String,
}
pub struct ValidationFailed {
	pub error: String,
}

// Message FSM container
pub struct EventMessage<S> {
	pub id: [u8; 32],
	pub timestamp: Instant,
	pub state: S,
}

// FSM Transitions with Result handling
impl EventMessage<Received> {
	pub fn new(raw: String) -> Self {
		let message = Self {
			id: generate_uuid(),
			timestamp: Instant::now(),
			state: Received { raw },
		};
		debug!("Message {:?} entering Received state", message.id);
		message
	}

	pub fn parse(self) -> Result<EventMessage<Parsed>, EventMessage<ParseFailed>> {
		info!("Message {:?} attempting transition from Received to Parsed", self.id);

		match serde_json::from_str::<Event>(&self.state.raw) {
			Ok(event) => {
				info!("Message {:?} successfully transitioned to Parsed state", self.id);
				Ok(EventMessage {
					id: self.id,
					timestamp: self.timestamp,
					state: Parsed { event },
				})
			}
			Err(e) => {
				error!("Message {:?} failed to parse, transitioning to ParseFailed: {}", self.id, e);
				Err(EventMessage {
					id: self.id,
					timestamp: self.timestamp,
					state: ParseFailed { error: e.to_string() },
				})
			}
		}
	}
}

impl EventMessage<Parsed> {
	pub fn validate(self) -> Result<EventMessage<Validated>, EventMessage<ValidationFailed>> {
		info!("Message {:?} attempting transition from Parsed to Validated", self.id);

		// Add your business logic validation here
		match &self.state.event {
			Event::Error { message } if message.is_empty() => {
				error!("Message {:?} validation failed: empty error message", self.id);
				Err(EventMessage {
					id: self.id,
					timestamp: self.timestamp,
					state: ValidationFailed {
						error: "Error event cannot have empty message".to_string(),
					},
				})
			}
			_ => {
				info!("Message {:?} successfully validated", self.id);
				Ok(EventMessage {
					id: self.id,
					timestamp: self.timestamp,
					state: Validated { event: self.state.event },
				})
			}
		}
	}
}

impl EventMessage<Validated> {
	pub fn queue(self) -> Result<EventMessage<Queued>, EventMessage<ValidationFailed>> {
		info!("Message {:?} transitioning from Validated to Queued", self.id);

		// Here you could add queue capacity checks, rate limiting, etc.
		Ok(EventMessage {
			id: self.id,
			timestamp: self.timestamp,
			state: Queued {
				event: self.state.event,
				queued_at: Instant::now(),
			},
		})
	}
}

impl EventMessage<Queued> {
	pub fn start_broadcast(self) -> Result<EventMessage<Broadcasting>, EventMessage<DeliveryFailed>> {
		info!("Message {:?} transitioning from Queued to Broadcasting", self.id);

		Ok(EventMessage {
			id: self.id,
			timestamp: self.timestamp,
			state: Broadcasting {
				event: self.state.event,
				started_at: Instant::now(),
			},
		})
	}
}

impl EventMessage<Broadcasting> {
	pub fn complete_delivery(self, delivered_count: usize, failed_count: usize) -> Result<EventMessage<Delivered>, EventMessage<DeliveryFailed>> {
		if delivered_count == 0 && failed_count > 0 {
			error!("Message {:?} delivery completely failed - {} failures, 0 delivered", self.id, failed_count);

			Err(EventMessage {
				id: self.id,
				timestamp: self.timestamp,
				state: DeliveryFailed {
					event: self.state.event,
					error: format!("All {} delivery attempts failed", failed_count),
					failed_at: Instant::now(),
				},
			})
		} else {
			info!("Message {:?} delivery completed - {} delivered, {} failed", self.id, delivered_count, failed_count);

			Ok(EventMessage {
				id: self.id,
				timestamp: self.timestamp,
				state: Delivered {
					event: self.state.event,
					delivered_count,
					failed_count,
					completed_at: Instant::now(),
				},
			})
		}
	}
}

#[derive(Clone)]
pub struct WebSocketFsm {
	// Active connections - only Connected state stored here
	connections: Arc<RwLock<HashMap<String, Connection<Connected>>>>,

	// Event broadcaster
	event_broadcast: broadcast::Sender<Event>,

	// Client count for metrics
	client_count: Arc<RwLock<usize>>,
}

impl WebSocketFsm {
	pub fn new() -> Self {
		let (event_broadcast, _) = broadcast::channel(1000);

		Self {
			connections: Arc::new(RwLock::new(HashMap::new())),
			event_broadcast,
			client_count: Arc::new(RwLock::new(0)),
		}
	}

	pub fn router(self) -> Router {
		Router::new().route("/ws", get(websocket_handler)).with_state(self)
	}

	// Process incoming message through FSM pipeline with proper error handling
	pub async fn process_message(&self, client_id: &str, raw_message: String) {
		let message = EventMessage::new(raw_message);
		let message_id = message.id;

		// Parse
		let parsed = match message.parse() {
			Ok(p) => p,
			Err(failed) => {
				error!("Message {:?} parse failed for client {}: {}", message_id, client_id, failed.state.error);
				self.send_error_to_client(client_id, &failed.state.error).await;
				return;
			}
		};

		// Handle pong separately - update connection state
		if matches!(parsed.state.event, Event::Pong) {
			if let Err(e) = self.update_client_ping(client_id).await {
				warn!("Failed to update ping for client {}: {}", client_id, e);
			}
			return;
		}

		// Validate
		let validated = match parsed.validate() {
			Ok(v) => v,
			Err(failed) => {
				error!("Message {:?} validation failed for client {}: {}", message_id, client_id, failed.state.error);
				self.send_error_to_client(client_id, &failed.state.error).await;
				return;
			}
		};

		// Queue
		let queued = match validated.queue() {
			Ok(q) => q,
			Err(failed) => {
				error!("Message {:?} queueing failed for client {}: {}", message_id, client_id, failed.state.error);
				return;
			}
		};

		// Start broadcasting
		let broadcasting = match queued.start_broadcast() {
			Ok(b) => b,
			Err(failed) => {
				error!("Message {:?} failed to start broadcast: {}", message_id, failed.state.error);
				return;
			}
		};

		// Execute the actual broadcast
		let broadcast_result = self.execute_broadcast(&broadcasting.state.event).await;

		// Complete the delivery based on results
		match broadcasting.complete_delivery(broadcast_result.delivered, broadcast_result.failed) {
			Ok(delivered) => {
				info!(
					"Message {:?} successfully delivered to {} clients, {} failures",
					message_id, delivered.state.delivered_count, delivered.state.failed_count
				);
			}
			Err(failed) => {
				error!("Message {:?} delivery failed: {}", message_id, failed.state.error);
			}
		}
	}

	// Execute the actual broadcast and return results
	async fn execute_broadcast(&self, event: &Event) -> BroadcastResult {
		match self.event_broadcast.send(event.clone()) {
			Ok(receiver_count) => {
				debug!("Event broadcasted to {} receivers", receiver_count);
				BroadcastResult {
					delivered: receiver_count,
					failed: 0,
				}
			}
			Err(e) => {
				error!("Failed to broadcast event: {}", e);
				BroadcastResult { delivered: 0, failed: 1 }
			}
		}
	}

	// Add new connection through FSM
	pub async fn add_connection(&self, sender: mpsc::Sender<Message>) -> Result<String, String> {
		let connecting = Connection::new();
		let client_id = connecting.id;

		let connected = connecting.establish(sender).map_err(|e| format!("Failed to establish connection: {}", e))?;

		// Store only Connected connections
		self.connections.write().await.insert(std::str::from_utf8(&client_id).unwrap().into(), connected);

		// Update count and broadcast
		let mut count = self.client_count.write().await;
		*count += 1;
		let new_count = *count;
		drop(count);

		// Broadcast client count update
		let _ = self.event_broadcast.send(Event::ClientCount { count: new_count });

		info!("Client {:?} connected successfully. Total: {}", client_id, new_count);
		Ok(std::str::from_utf8(&client_id).unwrap().into())
	}

	// Remove connection through FSM
	pub async fn remove_connection(&self, client_id: &str, reason: String) -> Result<(), String> {
		if let Some(connected) = self.connections.write().await.remove(client_id) {
			let _disconnected = connected.disconnect(reason).map_err(|e| format!("Failed to disconnect: {}", e))?;

			// Update count and broadcast
			let mut count = self.client_count.write().await;
			*count = count.saturating_sub(1);
			let new_count = *count;
			drop(count);

			let _ = self.event_broadcast.send(Event::ClientCount { count: new_count });

			info!("Client {} disconnected successfully. Total: {}", client_id, new_count);
		}
		Ok(())
	}

	// FSM-aware timeout checker
	pub fn start_timeout_monitor(&self, timeout: Duration) {
		let connections = self.connections.clone();
		let client_count = self.client_count.clone();
		let event_broadcast = self.event_broadcast.clone();

		tokio::spawn(async move {
			let mut interval = tokio::time::interval(Duration::from_secs(30));

			loop {
				interval.tick().await;
				let now = Instant::now();
				let mut stale_clients = Vec::new();

				// Find stale connections
				{
					let connections_guard = connections.read().await;
					for (client_id, connection) in connections_guard.iter() {
						if now.duration_since(connection.state.last_ping) > timeout {
							stale_clients.push(client_id.clone());
						}
					}
				}

				// Remove stale connections through FSM
				if !stale_clients.is_empty() {
					let removed_count = {
						let mut connections_guard = connections.write().await;
						let mut removed = 0;

						for client_id in stale_clients {
							if let Some(connected) = connections_guard.remove(&client_id) {
								match connected.mark_stale("Timeout".to_string()) {
									Ok(stale) => match stale.disconnect() {
										Ok(_disconnected) => {
											removed += 1;
											warn!("Client {} timed out and disconnected", client_id);
										}
										Err(e) => {
											error!("Failed to disconnect stale client {}: {}", client_id, e);
										}
									},
									Err(e) => {
										error!("Failed to mark client {} as stale: {}", client_id, e);
									}
								}
							}
						}
						removed
					};

					if removed_count > 0 {
						let mut count = client_count.write().await;
						*count = count.saturating_sub(removed_count);
						let new_count = *count;
						drop(count);

						let _ = event_broadcast.send(Event::ClientCount { count: new_count });
						info!("Removed {} timed out clients. Total: {}", removed_count, new_count);
					}
				}
			}
		});
	}

	// OBS Bridge - FSM compliant
	pub fn bridge_obs_events(&self, obs_client: Arc<obs_websocket::ObsWebSocketWithBroadcast>) {
		let event_broadcast = self.event_broadcast.clone();

		tokio::spawn(async move {
			let mut obs_receiver = obs_client.subscribe();
			info!("OBS event bridge started");

			loop {
				// TODO: remove the hardcoded value and impl configuration value
				match tokio::time::timeout(Duration::from_secs(45), obs_receiver.recv()).await {
					Ok(Ok(obs_event)) => {
						let event = Event::ObsStatus { status: obs_event };

						if let Err(e) = event_broadcast.send(event) {
							error!("Failed to broadcast OBS event: {}", e);
						} else {
							debug!("OBS event broadcasted to all clients");
						}
					}
					Ok(Err(e)) => match e {
						async_broadcast::RecvError::Closed => {
							error!("OBS receiver error: {}", e);
							break;
						}
						async_broadcast::RecvError::Overflowed(count) => {
							warn!("OBS receiver lagged behind by {} messages, continuing", count);
							continue;
						}
					},
					Err(_) => {
						warn!("OBS receiver timed out waiting for event");
						warn!("Is OBS connected {}", obs_client.is_connected().await);
						continue;
					}
				}
			}

			warn!("OBS event bridge ended");
		});
	}

	// Start the main broadcast loop
	pub fn start_broadcast_loop(&self) {
		let connections = self.connections.clone();
		let mut event_receiver = self.event_broadcast.subscribe();

		tokio::spawn(async move {
			while let Ok(event) = event_receiver.recv().await {
				// Why do we need a vec dst?
				let mut failed_clients = Vec::new();

				// Send to all connected clients
				{
					let connections_guard = connections.read().await;
					for (client_id, connection) in connections_guard.iter() {
						if let Err(e) = connection.send_event(&event).await {
							warn!("Failed to send to client {}: {}", client_id, e);
							failed_clients.push(client_id.clone());
						}
					}
				}

				// Remove failed clients through FSM
				if !failed_clients.is_empty() {
					let mut connections_guard = connections.write().await;
					for client_id in failed_clients {
						if let Some(connected) = connections_guard.remove(&client_id) {
							match connected.disconnect("Send failed".to_string()) {
								Ok(_disconnected) => {
									info!("Removed failed client: {}", client_id);
								}
								Err(e) => {
									error!("Failed to disconnect client {}: {}", client_id, e);
								}
							}
						}
					}
				}
			}
		});
	}

	async fn send_error_to_client(&self, client_id: &str, error: &str) {
		let connections = self.connections.read().await;
		if let Some(connection) = connections.get(client_id) {
			let error_event = Event::Error { message: error.to_string() };
			let _ = connection.send_event(&error_event).await;
		}
	}

	async fn update_client_ping(&self, client_id: &str) -> Result<(), String> {
		let mut connections = self.connections.write().await;
		if let Some(connection) = connections.get_mut(client_id) {
			connection.update_ping()
		} else {
			Err(format!("Client {} not found", client_id))
		}
	}

	pub async fn get_client_count(&self) -> usize {
		*self.client_count.read().await
	}
}

// Helper struct for broadcast results
#[derive(Debug)]
struct BroadcastResult {
	delivered: usize,
	failed: usize,
}

async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<WebSocketFsm>) -> impl IntoResponse {
	ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: WebSocketFsm) {
	let (mut sender, mut receiver) = socket.split();

	// Create client channel
	let (tx, mut rx) = mpsc::channel::<Message>(100);

	// Add connection through FSM
	let client_id = match state.add_connection(tx).await {
		Ok(id) => id,
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

	// Forward messages from channel to websocket
	let forward_task = {
		let client_id = client_id.clone();
		tokio::spawn(async move {
			while let Some(msg) = rx.recv().await {
				if let Err(e) = sender.send(msg).await {
					error!("Failed to forward message to client {}: {}", client_id, e);
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
					debug!("Received message from {}: {}", client_id, text);
					state.process_message(&client_id, text).await;
				}
				Message::Ping(_) => {
					if let Err(e) = state.update_client_ping(&client_id).await {
						warn!("Failed to update ping for {}: {}", client_id, e);
					}
				}
				Message::Pong(_) => {
					if let Err(e) = state.update_client_ping(&client_id).await {
						warn!("Failed to update pong for {}: {}", client_id, e);
					}
				}
				Message::Close(reason) => {
					info!("Client {} closed: {:?}", client_id, reason);
					break;
				}
				_ => {} // Ignore other message types
			},
			Err(e) => {
				error!("WebSocket error for {}: {}", client_id, e);
				break;
			}
		}
	}

	// Clean up through FSM
	if let Err(e) = state.remove_connection(&client_id, "Connection closed".to_string()).await {
		error!("Failed to remove connection {}: {}", client_id, e);
	}
	forward_task.abort();
}

pub async fn init_websocket() -> WebSocketFsm {
	let state = WebSocketFsm::new();

	// Start FSM processes
	state.start_broadcast_loop();
	state.start_timeout_monitor(Duration::from_secs(120));

	info!("FSM WebSocket system initialized");
	state
}

// Re-export for compatibility
pub use WebSocketFsm as WebSocketState;

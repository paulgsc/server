use crate::utils::{generate_uuid, string_to_buffer};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

// Client connection representation
#[derive(Debug)]
pub struct Client {
	pub id: [u8; 32],
	pub sender: mpsc::Sender<Message>,
	pub connected_at: Instant,
	pub last_ping: Instant,
	pub client_type: ClientType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientType {
	Viewer,  // Regular viewer client
	Control, // Admin/control client with extra permissions
}

// Message types for communication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum TimelineEvent {
	#[serde(rename = "SEGMENT_CHANGE")]
	SegmentChange {
		segment_id: String,
		panel_config: PanelConfig,
		transition: Option<Transition>,
	},
	#[serde(rename = "TIMELINE_START")]
	TimelineStart { timeline_id: String },
	#[serde(rename = "TIMELINE_PAUSE")]
	TimelinePause,
	#[serde(rename = "TIMELINE_STOP")]
	TimelineStop,
	#[serde(rename = "CLIENT_CONNECTED")]
	ClientConnected { client_id: [u8; 32] },
	#[serde(rename = "CLIENT_DISCONNECTED")]
	ClientDisconnected { client_id: [u8; 32] },
	#[serde(rename = "PING")]
	Ping,
	#[serde(rename = "PONG")]
	Pong,
	#[serde(rename = "ERROR")]
	Error { message: String },
}

// Panel configuration to send to clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelConfig {
	panels: HashMap<String, PanelSize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelSize {
	pub width_percent: f32,
	pub height_percent: f32,
	pub x_position: f32,
	pub y_position: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
	pub type_name: String,
	pub duration: u64, // milliseconds
}

// Application state
#[derive(Debug, Clone)]
pub struct WebSocketState {
	// Channel for broadcasting messages to all clients
	broadcaster: broadcast::Sender<TimelineEvent>,

	// Map of active clients
	clients: Arc<RwLock<HashMap<String, Client>>>,

	// Number of connected clients
	client_count: Arc<Mutex<usize>>,
}

impl WebSocketState {
	pub fn new() -> Self {
		// Create a broadcast channel with a reasonable capacity
		let (broadcaster, _) = broadcast::channel(100);

		Self {
			broadcaster,
			clients: Arc::new(RwLock::new(HashMap::new())),
			client_count: Arc::new(Mutex::new(0)),
		}
	}

	// Register with the app
	pub fn router(self) -> Router {
		Router::new().route("/ws", get(websocket_handler)).with_state(self)
	}

	// Broadcast a message to all connected clients
	pub async fn broadcast(&self, event: TimelineEvent) {
		if let Err(err) = self.broadcaster.send(event) {
			error!("Failed to broadcast message: {}", err);
		}
	}

	// Send a message to a specific client
	pub async fn send_to_client(&self, client_id: &str, event: TimelineEvent) -> bool {
		let clients = self.clients.read().await;

		if let Some(client) = clients.get(client_id) {
			let msg = serde_json::to_string(&event).unwrap_or_default();
			if let Err(err) = client.sender.send(Message::Text(msg)).await {
				error!("Failed to send message to client {}: {}", client_id, err);
				return false;
			}
			true
		} else {
			false
		}
	}

	// Get a count of connected clients
	pub async fn get_client_count(&self) -> usize {
		*self.client_count.lock().await
	}

	// Get a list of connected client IDs
	pub async fn get_connected_clients(&self) -> Vec<String> {
		let clients = self.clients.read().await;
		clients.keys().cloned().collect()
	}
}

// WebSocket connection handler
async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<WebSocketState>) -> impl IntoResponse {
	ws.on_upgrade(|socket| handle_socket(socket, state))
}

// Handle an individual WebSocket connection
async fn handle_socket(socket: WebSocket, state: WebSocketState) {
	// Split the socket into sender and receiver
	let (mut sender, mut receiver) = socket.split();

	// Generate a unique client ID
	let client_id = generate_uuid();

	// Create channels for this client
	let (tx, mut rx) = mpsc::channel::<Message>(100);

	// Subscribe to the broadcast channel
	let mut broadcast_rx = state.broadcaster.subscribe();

	// Store client information
	{
		let now = Instant::now();
		let client = Client {
			id: client_id.clone(),
			sender: tx,
			connected_at: now,
			last_ping: now,
			client_type: ClientType::Viewer, // Default as viewer
		};

		state.clients.write().await.insert(std::str::from_utf8(&client_id).unwrap().into(), client);

		// Increment client count
		let mut count = state.client_count.lock().await;
		*count += 1;

		info!("Client {} connected. Total clients: {}", std::str::from_utf8(&client_id).unwrap(), *count);

		// Notify about the new connection
		let _ = state.broadcast(TimelineEvent::ClientConnected { client_id: client_id.clone() }).await;
	}

	// Send initial ping
	if let Err(err) = sender.send(Message::Ping(vec![])).await {
		error!("Failed to send initial ping: {}", err);
	}

	// Set up ping interval
	let ping_state = state.clone();

	let ping_task = tokio::spawn(async move {
		let mut interval = interval(Duration::from_secs(30));

		loop {
			interval.tick().await;

			// Check if client still exists
			let clients = ping_state.clients.read().await;
			if !clients.contains_key(std::str::from_utf8(&client_id).unwrap()) {
				break;
			}

			// Send ping
			let ping_event = TimelineEvent::Ping;
			if !ping_state.send_to_client(std::str::from_utf8(&client_id).unwrap(), ping_event).await {
				break;
			}
		}
	});

	// Forward broadcast messages to this client
	let broadcast_state = state.clone();

	let broadcast_task = tokio::spawn(async move {
		while let Ok(event) = broadcast_rx.recv().await {
			let msg = match serde_json::to_string(&event) {
				Ok(msg) => Message::Text(msg),
				Err(err) => {
					error!("Failed to serialize event: {}", err);
					continue;
				}
			};

			let clients = broadcast_state.clients.read().await;
			if let Some(client) = clients.get(std::str::from_utf8(&client_id).unwrap()) {
				if let Err(err) = client.sender.send(msg).await {
					error!("Failed to forward broadcast message: {}", err);
					break;
				}
			} else {
				break;
			}
		}
	});

	// Task to forward messages from channel to WebSocket sender
	let forward_task = tokio::spawn(async move {
		while let Some(msg) = rx.recv().await {
			if let Err(err) = sender.send(msg).await {
				error!("Failed to forward message to WebSocket: {}", err);
				break;
			}
		}
	});

	// Process incoming messages
	while let Some(result) = receiver.next().await {
		match result {
			Ok(msg) => {
				match msg {
					Message::Text(text) => {
						debug!("Received text message from client {}: {}", std::str::from_utf8(&client_id).unwrap(), text);

						// Process the message
						match serde_json::from_str::<TimelineEvent>(&text) {
							Ok(event) => {
								match event {
									TimelineEvent::Pong => {
										// Update last ping time
										if let Some(client) = state.clients.write().await.get_mut(std::str::from_utf8(&client_id).unwrap()) {
											client.last_ping = Instant::now();
										}
									}
									// Handle other events as needed
									_ => {
										// Forward to all clients (including the sender)
										let _ = state.broadcast(event).await;
									}
								}
							}
							Err(err) => {
								warn!("Failed to parse message from client {}: {}", std::str::from_utf8(&client_id).unwrap(), err);

								// Send error message back to client
								let error_event = TimelineEvent::Error {
									message: format!("Invalid message format: {}", err),
								};
								let _ = state.send_to_client(std::str::from_utf8(&client_id).unwrap(), error_event).await;
							}
						}
					}
					Message::Binary(data) => {
						debug!("Received binary message from client {}: {} bytes", std::str::from_utf8(&client_id).unwrap(), data.len());
						// Handle binary message if needed
					}
					Message::Ping(data) => {
						debug!("Received ping from client {}", std::str::from_utf8(&client_id).unwrap());

						// Respond with pong
						if let Some(client) = state.clients.write().await.get_mut(std::str::from_utf8(&client_id).unwrap()) {
							client.last_ping = Instant::now();

							// Send pong response
							let _ = client.sender.send(Message::Pong(data)).await;
						}
					}
					Message::Pong(_) => {
						debug!("Received pong from client {}", std::str::from_utf8(&client_id).unwrap());

						// Update last ping time
						if let Some(client) = state.clients.write().await.get_mut(std::str::from_utf8(&client_id).unwrap()) {
							client.last_ping = Instant::now();
						}
					}
					Message::Close(reason) => {
						info!("Client {} closed connection: {:?}", std::str::from_utf8(&client_id).unwrap(), reason);
						break;
					}
				}
			}
			Err(err) => {
				error!("WebSocket error for client {}: {}", std::str::from_utf8(&client_id).unwrap(), err);
				break;
			}
		}
	}

	// Client disconnected, clean up
	{
		// Remove from clients map
		state.clients.write().await.remove(std::str::from_utf8(&client_id).unwrap());

		// Decrement client count
		let mut count = state.client_count.lock().await;
		*count = count.saturating_sub(1);

		info!("Client {} disconnected. Total clients: {}", std::str::from_utf8(&client_id).unwrap(), *count);

		// Notify about the disconnection
		let _ = state.broadcast(TimelineEvent::ClientDisconnected { client_id }).await;
	}

	// Abort the tasks
	ping_task.abort();
	broadcast_task.abort();
	forward_task.abort();
}

// Create a connection timeout checker
pub async fn start_client_timeout_checker(state: WebSocketState, timeout: Duration) {
	info!("Starting client timeout checker with timeout of {:?}", timeout);

	let mut interval = interval(Duration::from_secs(60));

	loop {
		interval.tick().await;

		let now = Instant::now();
		let mut disconnected_clients = Vec::new();

		// Find timed out clients
		{
			let clients = state.clients.read().await;

			for (client_id, client) in clients.iter() {
				if now.duration_since(client.last_ping) > timeout {
					warn!("Client {} timed out after {:?}", client_id, timeout);
					disconnected_clients.push(client_id.clone());
				}
			}
		}

		// Remove timed out clients
		{
			let mut clients = state.clients.write().await;

			for client_id in &disconnected_clients {
				clients.remove(client_id);
			}

			if !disconnected_clients.is_empty() {
				let mut count = state.client_count.lock().await;
				*count = count.saturating_sub(disconnected_clients.len());

				info!("Removed {} timed out clients. Total clients: {}", disconnected_clients.len(), *count);
			}
		}

		// Notify about disconnections
		for client_id in disconnected_clients {
			let _ = state
				.broadcast(TimelineEvent::ClientDisconnected {
					client_id: string_to_buffer(&client_id),
				})
				.await;
		}
	}
}

// Helper function to initialize the WebSocket system
pub async fn init_websocket() -> WebSocketState {
	let state = WebSocketState::new();

	// Start timeout checker in a separate task
	let timeout_state = state.clone();
	tokio::spawn(async move {
		start_client_timeout_checker(timeout_state, Duration::from_secs(120)).await;
	});

	info!("WebSocket system initialized");
	state
}

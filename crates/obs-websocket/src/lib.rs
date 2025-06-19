// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It separates core OBS logic from context-specific concerns (broadcasting, etc.)

mod auth;
mod messages;

use auth::authenticate;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{
	sink::SinkExt,
	stream::{SplitSink, SplitStream, StreamExt},
};
use messages::{fetch_init_state, process_obs_message, ObsEvent};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::interval;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream, WebSocketStream};
use tracing::{error, info, warn};

/// Configuration for the OBS WebSocket connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsConfig {
	pub host: String,
	pub port: u16,
	pub password: String,
}

/// Current status of OBS
// TODO: THIS STRUCT MAYBE BETTER NAME, MAYBE WE ALREADY GET THIS FIELDS FOR FREE!
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsStatus {
	pub streaming: bool,
	pub recording: bool,
	pub stream_timecode: String,
	pub recording_timecode: String,
	pub scenes: Vec<String>,
	pub current_scene: String,
}

impl Default for ObsConfig {
	fn default() -> Self {
		Self {
			host: "10.0.0.25".to_string(),
			port: 4455,
			password: "pwd".to_string(),
		}
	}
}

impl Default for ObsStatus {
	fn default() -> Self {
		Self {
			streaming: false,
			recording: false,
			stream_timecode: "00:00:00.000".to_string(),
			recording_timecode: "00:00:00.000".to_string(),
			scenes: Vec::new(),
			current_scene: "".to_string(),
		}
	}
}

impl ObsStatus {
	/// Check if two status instances are different enough to warrant broadcasting
	pub fn has_meaningful_changes(&self, other: &ObsStatus) -> bool {
		self.streaming != other.streaming || self.recording != other.recording || self.current_scene != other.current_scene || self.scenes != other.scenes
		// Note: We don't compare timecodes as they change frequently
	}
}

/// Core OBS WebSocket client - usable by both daemon and Axum server
pub struct ObsWebSocket {
	config: Arc<RwLock<ObsConfig>>,
	status: Arc<RwLock<ObsStatus>>,
	_stream: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
	sink: Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>>,
	stream_part: Option<SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
}

impl ObsWebSocket {
	/// Create a new OBS WebSocket client
	#[must_use]
	pub fn new(config: ObsConfig) -> Self {
		Self {
			config: Arc::new(RwLock::new(config)),
			status: Arc::new(RwLock::new(ObsStatus::default())),
			_stream: None,
			sink: None,
			stream_part: None,
		}
	}

	/// Connect to OBS WebSocket
	pub async fn connect(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let config = self.config.read().await;
		let url = format!("ws://{}:{}", config.host, config.port);

		info!("Connecting to OBS WebSocket at {}", url);

		let (ws_stream, _) = connect_async(url).await?;
		let (sink, stream) = ws_stream.split();

		self.sink = Some(sink);
		self.stream_part = Some(stream);

		// Handle authentication
		if let (Some(ref mut sink), Some(ref mut stream)) = (&mut self.sink, &mut self.stream_part) {
			authenticate(&config.password, sink, stream).await?;
			fetch_init_state(sink).await?;
		}

		info!("Connected to OBS WebSocket");
		Ok(())
	}

	/// Get the next event from OBS
	pub async fn next_event(&mut self) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
		if let Some(ref mut stream) = self.stream_part {
			while let Some(message) = stream.next().await {
				match message? {
					TungsteniteMessage::Text(text) => {
						if let Ok(event) = process_obs_message(text.to_string(), self.status.clone()).await {
							return Ok(event);
						}
					}
					TungsteniteMessage::Close(_) => {
						return Err("Connection closed".into());
					}
					_ => continue,
				}
			}
		}
		Err("No stream available".into())
	}

	/// Get current status
	pub async fn get_status(&self) -> ObsStatus {
		self.status.read().await.clone()
	}

	/// Get status reference for sharing
	pub fn get_status_ref(&self) -> Arc<RwLock<ObsStatus>> {
		Arc::clone(&self.status)
	}

	/// Get current config
	pub async fn get_config(&self) -> ObsConfig {
		self.config.read().await.clone()
	}

	/// Update configuration
	pub async fn update_config(&mut self, config: ObsConfig) {
		*self.config.write().await = config;
	}

	/// Send a request to OBS
	pub async fn send_request(&mut self, request: serde_json::Value) -> Result<(), Box<dyn Error + Send + Sync>> {
		if let Some(ref mut sink) = self.sink {
			sink.send(TungsteniteMessage::Text(request.to_string().into())).await?;
			sink.flush().await?;
		}
		Ok(())
	}

	/// Start status polling
	pub async fn start_status_polling(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let mut interval = interval(Duration::from_secs(1));
		let mut request_id = 0;

		loop {
			interval.tick().await;
			request_id += 1;

			// Request stream status
			let status_req = json!({
				"op": 6,
				"d": {
					"requestType": "GetStreamStatus",
					"requestId": format!("stream-{}", request_id)
				}
			});

			if let Err(e) = self.send_request(status_req).await {
				error!("Failed to send stream status request: {}", e);
				break;
			}

			// Request recording status
			let recording_req = json!({
				"op": 6,
				"d": {
					"requestType": "GetRecordStatus",
					"requestId": format!("recording-{}", request_id)
				}
			});

			if let Err(e) = self.send_request(recording_req).await {
				error!("Failed to send recording status request: {}", e);
				break;
			}
		}

		Ok(())
	}

	/// Check if connection is alive
	pub fn is_connected(&self) -> bool {
		self.sink.is_some() && self.stream_part.is_some()
	}
}

/// Axum-specific wrapper that adds broadcasting functionality
pub struct ObsWebSocketWithBroadcast {
	inner: ObsWebSocket,
	broadcaster: async_broadcast::Sender<ObsStatus>,
	_receiver: async_broadcast::Receiver<ObsStatus>,
}

impl ObsWebSocketWithBroadcast {
	/// Create new broadcaster-enabled OBS client
	pub fn new(config: ObsConfig) -> Self {
		let (sender, receiver) = async_broadcast::broadcast(100);
		Self {
			inner: ObsWebSocket::new(config),
			broadcaster: sender,
			_receiver: receiver,
		}
	}

	/// Connect to OBS
	pub async fn connect(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.connect().await
	}

	/// Handle the next event and broadcast status updates
	pub async fn handle_next_event(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let event = self.inner.next_event().await?;

		// Broadcast if the event should trigger an update
		if event.should_broadcast() {
			let status = self.inner.get_status().await;
			if let Err(e) = self.broadcaster.broadcast(status).await {
				error!("Failed to broadcast status update: {}", e);
			}
		}

		Ok(())
	}

	/// Get current status
	pub async fn get_status(&self) -> ObsStatus {
		self.inner.get_status().await
	}

	/// Get status reference
	pub fn get_status_ref(&self) -> Arc<RwLock<ObsStatus>> {
		self.inner.get_status_ref()
	}

	/// Update configuration
	pub async fn update_config(&mut self, config: ObsConfig) {
		self.inner.update_config(config).await;
	}

	/// Get a new receiver for status updates
	pub fn subscribe(&self) -> async_broadcast::Receiver<ObsStatus> {
		self.broadcaster.new_receiver()
	}

	/// Create WebSocket handler for Axum
	pub fn websocket_handler(&self, ws: WebSocketUpgrade) -> impl IntoResponse {
		let broadcaster = self.broadcaster.clone();
		let status = self.inner.get_status_ref();

		ws.on_upgrade(move |socket| async move {
			handle_socket(socket, status, broadcaster).await;
		})
	}

	/// Start the client with automatic reconnection and broadcasting
	pub fn start(&self) {
		let broadcaster = self.broadcaster.clone();
		let config = self.inner.config.clone();
		let status = self.inner.status.clone();

		tokio::spawn(async move {
			obs_websocket_client_with_broadcast(config, status, broadcaster).await;
		});
	}
}

// Private implementation for Axum WebSocket handling
async fn handle_socket(socket: WebSocket, status: Arc<RwLock<ObsStatus>>, broadcaster: async_broadcast::Sender<ObsStatus>) {
	let status_snapshot = status.read().await.clone();
	let (mut sender, mut receiver) = socket.split();

	// Send initial status
	match serde_json::to_string(&status_snapshot) {
		Ok(res) => {
			if let Err(e) = sender.send(Message::Text(res)).await {
				error!("Error sending initial status: {}", e);
				return;
			}
			info!("Sent initial status!");
		}
		Err(e) => {
			error!("Serialization failed with error: {}", e);
			return;
		}
	}

	let mut rx = broadcaster.new_receiver();

	let send_task = tokio::spawn(async move {
		loop {
			match rx.recv().await {
				Ok(status) => match serde_json::to_string(&status) {
					Ok(json) => {
						if sender.send(Message::Text(json)).await.is_err() {
							break;
						}
						info!("Sent status update!");
					}
					Err(e) => error!("Failed to serialize status: {}", e),
				},
				Err(async_broadcast::RecvError::Closed) => {
					error!("Broadcaster closed");
					break;
				}
				Err(async_broadcast::RecvError::Overflowed(_)) => {
					error!("Missed messages");
					continue;
				}
			}
		}
	});

	let recv_task = tokio::spawn(async move {
		while let Some(Ok(msg)) = receiver.next().await {
			match msg {
				Message::Text(text) => {
					info!("Received message: {}", text);
				}
				Message::Close(frame) => {
					warn!("Client requested close: {:?}", frame);
					break;
				}
				_ => {}
			}
		}
	});

	let _ = tokio::join!(send_task, recv_task);
}

// Background task for broadcast-enabled client
async fn obs_websocket_client_with_broadcast(config: Arc<RwLock<ObsConfig>>, status: Arc<RwLock<ObsStatus>>, broadcaster: async_broadcast::Sender<ObsStatus>) {
	info!("Started OBS WebSocket with broadcast...");
	let mut reconnect_interval = interval(Duration::from_secs(5));

	loop {
		reconnect_interval.tick().await;

		let config_snapshot = config.read().await.clone();
		let mut obs_client = ObsWebSocket::new(config_snapshot);

		match obs_client.connect().await {
			Ok(_) => {
				info!("Connected to OBS WebSocket");

				// Start status polling in background
				let mut polling_client = ObsWebSocket::new(obs_client.get_config().await);
				let _ = polling_client.connect().await;

				tokio::spawn(async move {
					let _ = polling_client.start_status_polling().await;
				});

				// Main event loop
				loop {
					match obs_client.next_event().await {
						Ok(event) => {
							if event.should_broadcast() {
								let status_snapshot = status.read().await.clone();
								if let Err(e) = broadcaster.broadcast(status_snapshot).await {
									error!("Failed to broadcast status: {}", e);
								}
							}
						}
						Err(e) => {
							error!("OBS WebSocket error: {}", e);
							break;
						}
					}
				}
			}
			Err(e) => {
				error!("Failed to connect to OBS WebSocket: {}", e);
			}
		}

		warn!("OBS connection lost, reconnecting in 5 seconds...");
	}
}

/// Simple client factory for daemon use
pub fn create_obs_client(config: ObsConfig) -> ObsWebSocket {
	ObsWebSocket::new(config)
}

/// Broadcast-enabled client factory for Axum use
pub fn create_obs_client_with_broadcast(config: ObsConfig) -> ObsWebSocketWithBroadcast {
	ObsWebSocketWithBroadcast::new(config)
}

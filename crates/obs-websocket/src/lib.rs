// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It separates core OBS logic from context-specific concerns (broadcasting, etc.)

mod auth;
mod messages;
mod polling;

use auth::authenticate;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{sink::SinkExt, stream::StreamExt};
use messages::{fetch_init_state, process_obs_message, ObsEvent, ObsStats};
use polling::{ObsPollingManager, ObsRequestBuilder};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as TungsteniteMessage};
use tracing::{error, info, warn};

/// Configuration for the OBS WebSocket connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsConfig {
	pub host: String,
	pub port: u16,
	pub password: String,
}

/// Current status of OBS - Extended with more comprehensive state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsStatus {
	pub streaming: bool,
	pub recording: bool,
	pub stream_timecode: String,
	pub recording_timecode: String,
	pub scenes: Vec<String>,
	pub current_scene: String,
	pub sources: Vec<String>,
	pub inputs: Vec<String>,
	pub virtual_camera: bool,
	pub replay_buffer: bool,
	pub studio_mode: bool,
	pub current_profile: String,
	pub current_collection: String,
	pub current_transition: String,
	pub version: String,
	pub stats: ObsStats,
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
			sources: Vec::new(),
			inputs: Vec::new(),
			virtual_camera: false,
			replay_buffer: false,
			studio_mode: false,
			current_profile: "".to_string(),
			current_collection: "".to_string(),
			current_transition: "".to_string(),
			version: "".to_string(),
			stats: ObsStats::default(),
		}
	}
}

impl ObsStatus {
	/// Check if two status instances are different enough to warrant broadcasting
	pub fn has_meaningful_changes(&self, other: &ObsStatus) -> bool {
		self.streaming != other.streaming
			|| self.recording != other.recording
			|| self.current_scene != other.current_scene
			|| self.scenes != other.scenes
			|| self.sources != other.sources
			|| self.inputs != other.inputs
			|| self.virtual_camera != other.virtual_camera
			|| self.replay_buffer != other.replay_buffer
			|| self.studio_mode != other.studio_mode
			|| self.current_profile != other.current_profile
			|| self.current_collection != other.current_collection
			|| self.current_transition != other.current_transition
		// Note: We don't compare timecodes and stats as they change frequently
	}
}

pub enum OBSCommand {
	SendRequest(serde_json::Value),
	Disconnect,
}

/// Core OBS WebSocket client - usable by both daemon and Axum server
pub struct ObsWebSocket {
	config: Arc<RwLock<ObsConfig>>,
	status: Arc<RwLock<ObsStatus>>,
	command_tx: Option<tokio::sync::mpsc::UnboundedSender<OBSCommand>>,
	event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<ObsEvent>>,
	task_handle: Option<tokio::task::JoinHandle<()>>,
	request_builder: ObsRequestBuilder,
}

impl ObsWebSocket {
	/// Create a new OBS WebSocket client
	#[must_use]
	pub fn new(config: ObsConfig) -> Self {
		Self {
			config: Arc::new(RwLock::new(config)),
			status: Arc::new(RwLock::new(ObsStatus::default())),
			command_tx: None,
			event_rx: None,
			task_handle: None,
			request_builder: ObsRequestBuilder::new(),
		}
	}

	/// Connect with comprehensive polling using the new polling module
	pub async fn connect(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let config = self.config.read().await;
		let url = format!("ws://{}:{}", config.host, config.port);

		info!("Connecting to OBS WebSocket at {}", url);

		let (ws_stream, _) = connect_async(url).await?;
		let (mut sink, mut stream) = ws_stream.split();

		// Handle authentication
		authenticate(&config.password, &mut sink, &mut stream).await?;
		fetch_init_state(&mut sink).await?;

		// Create channels for communication
		let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<OBSCommand>();
		let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<ObsEvent>();

		let status = Arc::clone(&self.status);

		// Start the comprehensive polling manager
		let polling_manager = ObsPollingManager::new();
		let polling_task = tokio::spawn(async move {
			polling_manager.start_polling_loop(sink, cmd_rx).await;
		});

		let message_task = tokio::spawn(async move {
			while let Some(msg) = stream.next().await {
				match msg {
					Ok(TungsteniteMessage::Text(text)) => {
						if let Ok(event) = process_obs_message(text.to_string(), status.clone()).await {
							let _ = event_tx.send(event);
						}
					}
					Ok(TungsteniteMessage::Close(_)) => {
						error!("Connection closed");
						break;
					}
					Err(e) => {
						error!("WebSocket error: {}", e);
						break;
					}
					_ => continue,
				}
			}
		});

		// Combine both tasks
		let combined_task = tokio::spawn(async move {
			tokio::select! {
				_ = polling_task => {
					error!("Polling task ended");
				}
				_ = message_task => {
					error!("Message processing task ended");
				}
			}
		});

		self.command_tx = Some(cmd_tx);
		self.event_rx = Some(event_rx);
		self.task_handle = Some(combined_task);

		info!("Connected to OBS WebSocket with comprehensive polling");
		Ok(())
	}

	/// Get the next event from OBS
	pub async fn next_event(&mut self) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
		if let Some(ref mut rx) = self.event_rx {
			if let Some(event) = rx.recv().await {
				return Ok(event);
			}
		}
		Err("No event receiver available or connection closed".into())
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
		if let Some(ref tx) = self.command_tx {
			tx.send(OBSCommand::SendRequest(request))?;
			Ok(())
		} else {
			Err("Not connected".into())
		}
	}

	/// Start streaming
	pub async fn start_stream(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.start_stream();
		self.send_request(request).await
	}

	/// Stop streaming
	pub async fn stop_stream(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.stop_stream();
		self.send_request(request).await
	}

	/// Start recording
	pub async fn start_recording(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.start_recording();
		self.send_request(request).await
	}

	/// Stop recording
	pub async fn stop_recording(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.stop_recording();
		self.send_request(request).await
	}

	/// Switch to a specific scene
	pub async fn switch_scene(&mut self, scene_name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.switch_scene(scene_name);
		self.send_request(request).await
	}

	/// Mute/unmute audio source
	pub async fn set_input_mute(&mut self, input_name: &str, muted: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.set_input_mute(input_name, muted);
		self.send_request(request).await
	}

	/// Set audio volume
	pub async fn set_input_volume(&mut self, input_name: &str, volume: f64) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.set_input_volume(input_name, volume);
		self.send_request(request).await
	}

	/// Toggle studio mode
	pub async fn toggle_studio_mode(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.toggle_studio_mode();
		self.send_request(request).await
	}

	/// Toggle virtual camera
	pub async fn toggle_virtual_camera(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.toggle_virtual_camera();
		self.send_request(request).await
	}

	/// Toggle replay buffer
	pub async fn toggle_replay_buffer(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.toggle_replay_buffer();
		self.send_request(request).await
	}

	/// Check if connection is alive
	pub fn is_connected(&self) -> bool {
		self.command_tx.is_some() && self.event_rx.is_some() && self.task_handle.as_ref().map_or(false, |h| !h.is_finished())
	}

	/// Disconnect and cleanup
	pub async fn disconnect(&mut self) {
		if let Some(tx) = self.command_tx.take() {
			let _ = tx.send(OBSCommand::Disconnect);
		}

		if let Some(handle) = self.task_handle.take() {
			let _ = handle.await;
		}

		self.event_rx = None;
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

	/// Start the event handling loop with broadcasting
	pub fn start(&self) {
		let broadcaster = self.broadcaster.clone();
		let status_ref = self.inner.get_status_ref();
		let config = self.inner.config.clone();

		tokio::spawn(async move {
			loop {
				let mut obs_client = ObsWebSocket::new(config.read().await.clone());

				match obs_client.connect().await {
					Ok(_) => {
						info!("Connected to OBS WebSocket");

						// Event handling loop
						while obs_client.is_connected() {
							match obs_client.next_event().await {
								Ok(event) => {
									if event.should_broadcast() {
										let status = status_ref.read().await.clone();
										if let Err(e) = broadcaster.broadcast(status).await {
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
				tokio::time::sleep(Duration::from_secs(5)).await;
			}
		});
	}

	/// Forward convenience methods to inner client
	pub async fn start_stream(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.start_stream().await
	}

	pub async fn stop_stream(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.stop_stream().await
	}

	pub async fn start_recording(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.start_recording().await
	}

	pub async fn stop_recording(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.stop_recording().await
	}

	pub async fn switch_scene(&mut self, scene_name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.switch_scene(scene_name).await
	}

	pub async fn set_input_mute(&mut self, input_name: &str, muted: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.set_input_mute(input_name, muted).await
	}

	pub async fn set_input_volume(&mut self, input_name: &str, volume: f64) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.set_input_volume(input_name, volume).await
	}

	pub async fn toggle_studio_mode(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.toggle_studio_mode().await
	}

	pub async fn toggle_virtual_camera(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.toggle_virtual_camera().await
	}

	pub async fn toggle_replay_buffer(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.toggle_replay_buffer().await
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

/// Simple client factory for daemon use
pub fn create_obs_client(config: ObsConfig) -> ObsWebSocket {
	ObsWebSocket::new(config)
}

/// Broadcast-enabled client factory for Axum use
pub fn create_obs_client_with_broadcast(config: ObsConfig) -> ObsWebSocketWithBroadcast {
	ObsWebSocketWithBroadcast::new(config)
}

// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It separates core OBS logic from context-specific concerns (broadcasting, etc.)

mod auth;
mod messages;
mod polling;

pub use messages::ObsEvent;
pub use polling::{ObsRequestType, PollingFrequency};

use auth::authenticate;
use axum::{
	extract::{ws::WebSocketUpgrade, State},
	response::IntoResponse,
};
use futures_util::stream::StreamExt;
use messages::{fetch_init_state, process_obs_message};
use polling::{ObsPollingManager, ObsRequestBuilder};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
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

impl Default for ObsConfig {
	fn default() -> Self {
		Self {
			host: "10.0.0.25".to_string(),
			port: 4455,
			password: "pwd".to_string(),
		}
	}
}

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
	/// Maximum number of consecutive failures before giving up
	pub max_consecutive_failures: usize,
	/// Initial delay between retries
	pub initial_delay: Duration,
	/// Maximum delay between retries (for exponential backoff)
	pub max_delay: Duration,
	/// Multiplier for exponential backoff
	pub backoff_multiplier: f64,
	/// How long to wait after max failures before trying again
	pub circuit_breaker_timeout: Duration,
}

impl Default for RetryConfig {
	fn default() -> Self {
		Self {
			max_consecutive_failures: 10,
			initial_delay: Duration::from_secs(1),
			max_delay: Duration::from_secs(60),
			backoff_multiplier: 1.5,
			circuit_breaker_timeout: Duration::from_secs(300), // 5 minutes
		}
	}
}

#[derive(Debug)]
enum RetryState {
	/// Normal operation, will retry on failure
	Active { consecutive_failures: usize, current_delay: Duration },
	/// Too many failures, circuit breaker is open
	CircuitOpen { opened_at: Instant },
}

pub enum OBSCommand {
	SendRequest(serde_json::Value),
	Disconnect,
}

/// Core OBS WebSocket client - usable by both daemon and Axum server
pub struct ObsWebSocket {
	config: Arc<RwLock<ObsConfig>>,
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
			command_tx: None,
			event_rx: None,
			task_handle: None,
			request_builder: ObsRequestBuilder::new(),
		}
	}

	/// Connect with comprehensive polling using the new polling module
	pub async fn connect(&mut self, r: &[(ObsRequestType, PollingFrequency)]) -> Result<(), Box<dyn Error + Send + Sync>> {
		let config = self.config.read().await;
		let url = format!("ws://{}:{}", config.host, config.port);

		info!("Connecting to OBS WebSocket at {}", url);

		let (ws_stream, _) = connect_async(url).await?;
		let (mut sink, mut stream) = ws_stream.split();

		// Handle authentication
		authenticate(&config.password, &mut sink, &mut stream).await?;
		info!("Sucessfully connected!");
		fetch_init_state(&mut sink).await?;

		// Create channels for communication
		let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<OBSCommand>();
		let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<ObsEvent>();

		// Start the comprehensive polling manager
		let polling_manager = ObsPollingManager::from_request_slice(r);
		let polling_task = tokio::spawn(async move {
			polling_manager.start_polling_loop(sink, cmd_rx).await;
		});

		let message_task = tokio::spawn(async move {
			let mut consecutive_failures = 0;
			const MAX_CONSECUTIVE_FAILURES: usize = 10;

			while let Some(msg) = stream.next().await {
				match msg {
					Ok(TungsteniteMessage::Text(text)) => match process_obs_message(text.to_string()).await {
						Ok(event) => {
							consecutive_failures = 0;
							if let Err(e) = event_tx.send(event) {
								error!("Failed to send event channel: {}", e);
								break;
							}
						}
						Err(e) => {
							consecutive_failures += 1;
							warn!("Failed to process OBS message (failure {}/{}): {}", consecutive_failures, MAX_CONSECUTIVE_FAILURES, e);
							warn!("Raw message that failed: {}", text);

							// If we have too many consecutive failures, something is seriously wrong
							if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
								error!("Too many consecutive message processing failures. Breaking connection.");
								break;
							}

							continue;
						}
					},
					Ok(TungsteniteMessage::Close(_)) => {
						warn!("Connection closed");
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
			match tokio::time::timeout(Duration::from_secs(30), rx.recv()).await {
				Ok(Some(event)) => {
					return Ok(event);
				}
				Ok(None) => {
					error!("Event channel closed - OBS connection lost");
					return Err("Event channel closed".into());
				}
				Err(_) => {
					warn!("Timeout waiting for OBS events - connection may be stuck");
					return Err("Timeout waiting for events".into());
				}
			}
		}
		Err("No event receiver available or connection closed".into())
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
	broadcaster: async_broadcast::Sender<ObsEvent>,
	_receiver: async_broadcast::Receiver<ObsEvent>,
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
	pub async fn connect(&mut self, r: &[(ObsRequestType, PollingFrequency)]) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.connect(r).await
	}

	/// Handle the next event and broadcast status updates
	pub async fn handle_next_event(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let event = self.inner.next_event().await?;

		// Broadcast if the event should trigger an update
		if event.should_broadcast() {
			if let Err(e) = self.broadcaster.broadcast(event).await {
				error!("Failed to broadcast status update: {}", e);
			}
		}

		Ok(())
	}

	/// Update configuration
	pub async fn update_config(&mut self, config: ObsConfig) {
		self.inner.update_config(config).await;
	}

	/// Get a new receiver for status updates
	pub fn subscribe(&self) -> async_broadcast::Receiver<ObsEvent> {
		self.broadcaster.new_receiver()
	}

	/// Create WebSocket handler for Axum
	pub fn websocket_handler(&self, ws: WebSocketUpgrade, State(obs_client): State<Arc<ObsWebSocketWithBroadcast>>) -> impl IntoResponse {
		let obs_receiver = obs_client.subscribe();

		ws.on_upgrade(move |socket| async move {
			handle_obs_socket(socket, obs_receiver).await;
		})
	}

	/// Start the event handling loop with broadcasting
	pub fn start(&self, requests: Box<[(ObsRequestType, PollingFrequency)]>, retry_config: RetryConfig) -> BroadcastHandle {
		let broadcaster = self.broadcaster.clone();
		let broadcaster_for_task = self.broadcaster.clone();
		let config = self.inner.config.clone();

		let handle = tokio::spawn(async move {
			let mut retry_state = RetryState::Active {
				consecutive_failures: 0,
				current_delay: retry_config.initial_delay,
			};

			loop {
				info!("retry state: {:?}", retry_state);
				match retry_state {
					RetryState::Active {
						consecutive_failures,
						current_delay,
					} => {
						let mut obs_client = ObsWebSocket::new(config.read().await.clone());
						match obs_client.connect(&requests).await {
							Ok(_) => {
								info!("Connected to OBS WebSocket (attempt {} successful)", consecutive_failures + 1);

								// Reset retry state on successful connection
								retry_state = RetryState::Active {
									consecutive_failures: 0,
									current_delay: retry_config.initial_delay,
								};

								// Event handling loop
								while obs_client.is_connected() {
									match obs_client.next_event().await {
										Ok(event) => {
											info!("Received new event: Attempting to broadcast!");
											if event.should_broadcast() {
												if let Err(e) = broadcaster_for_task.broadcast(event).await {
													error!("Failed to broadcast event: {}", e);
												}
												info!("Successfully broadcasted!");
											}
										}
										Err(e) => {
											warn!("OBS WebSocket event error: {}", e);
											break; // Will trigger reconnect
										}
									}
								}

								// Connection lost, but don't count as failure since we were connected
								warn!("OBS connection lost, will attempt to reconnect");
							}
							Err(e) => {
								let new_failures = consecutive_failures + 1;
								error!("Failed to connect to OBS WebSocket (attempt {}): {}", new_failures, e);

								if new_failures >= retry_config.max_consecutive_failures {
									error!(
										"Max consecutive failures ({}) reached. Opening circuit breaker for {} seconds",
										retry_config.max_consecutive_failures,
										retry_config.circuit_breaker_timeout.as_secs()
									);

									retry_state = RetryState::CircuitOpen { opened_at: Instant::now() };
									continue; // Skip the delay, go straight to circuit breaker logic
								} else {
									// Calculate next delay with exponential backoff
									let next_delay = Duration::from_millis((current_delay.as_millis() as f64 * retry_config.backoff_multiplier) as u64).min(retry_config.max_delay);

									warn!(
										"Will retry in {} seconds (failure {}/{})",
										current_delay.as_secs(),
										new_failures,
										retry_config.max_consecutive_failures
									);

									retry_state = RetryState::Active {
										consecutive_failures: new_failures,
										current_delay: next_delay,
									};

									tokio::time::sleep(current_delay).await;
								}
							}
						}
					}

					RetryState::CircuitOpen { opened_at } => {
						if opened_at.elapsed() >= retry_config.circuit_breaker_timeout {
							info!("Circuit breaker timeout elapsed, attempting to reconnect to OBS");
							retry_state = RetryState::Active {
								consecutive_failures: 0,
								current_delay: retry_config.initial_delay,
							};
						} else {
							// Wait a bit before checking again
							warn!("wait 30s before next retry!");
							tokio::time::sleep(Duration::from_secs(30)).await;
						}
					}
				}
			}
		});
		warn!("We exited loop for some reason!");

		BroadcastHandle { task_handle: handle, broadcaster }
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
async fn handle_obs_socket(
	socket: axum::extract::ws::WebSocket,
	mut obs_receiver: async_broadcast::Receiver<ObsEvent>, // THIS WAS MISSING!
) {
	use axum::extract::ws::Message;
	use futures_util::{sink::SinkExt, stream::StreamExt};

	let (mut sender, mut receiver) = socket.split();

	// Task to handle sending OBS updates to WebSocket client
	let send_task = tokio::spawn(async move {
		while let Ok(event) = obs_receiver.recv().await {
			match serde_json::to_string(&event) {
				Ok(json) => {
					if let Err(e) = sender.send(Message::Text(json)).await {
						tracing::error!("Failed to send OBS status update: {}", e);
						break;
					}
					tracing::info!("Sent OBS status update to WebSocket client");
				}
				Err(e) => {
					tracing::error!("Failed to serialize OBS status: {}", e);
				}
			}
		}
	});

	// Task to handle incoming WebSocket messages
	let recv_task = tokio::spawn(async move {
		while let Some(msg_result) = receiver.next().await {
			match msg_result {
				Ok(Message::Text(text)) => {
					tracing::info!("Received WebSocket message: {}", text);
					// Handle client commands here if needed
				}
				Ok(Message::Close(frame)) => {
					tracing::info!("WebSocket client requested close: {:?}", frame);
					break;
				}
				Ok(_) => {} // Handle other message types if needed
				Err(e) => {
					tracing::error!("WebSocket error: {}", e);
					break;
				}
			}
		}
	});

	// Wait for either task to complete
	let _ = tokio::join!(send_task, recv_task);
	tracing::info!("WebSocket connection closed");
}

/// Simple client factory for daemon use
pub fn create_obs_client(config: ObsConfig) -> ObsWebSocket {
	ObsWebSocket::new(config)
}

/// Broadcast-enabled client factory for Axum use
pub fn create_obs_client_with_broadcast(config: ObsConfig) -> ObsWebSocketWithBroadcast {
	ObsWebSocketWithBroadcast::new(config)
}

pub struct BroadcastHandle {
	task_handle: tokio::task::JoinHandle<()>,
	broadcaster: async_broadcast::Sender<ObsEvent>,
}

impl BroadcastHandle {
	/// Get a new receiver for status updates
	pub fn subscribe(&self) -> async_broadcast::Receiver<ObsEvent> {
		self.broadcaster.new_receiver()
	}

	/// Stop the background task
	pub async fn stop(self) {
		self.task_handle.abort();
		let _ = self.task_handle.await;
	}

	/// Check if the background task is still running
	pub fn is_running(&self) -> bool {
		!self.task_handle.is_finished()
	}
}

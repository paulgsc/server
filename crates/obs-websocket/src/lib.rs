// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It separates core OBS logic from context-specific concerns (broadcasting, etc.)

use futures_util::sink::SinkExt;
mod auth;
mod messages;
mod polling;

pub use messages::ObsEvent;
pub use polling::{ObsRequestType, PollingFrequency};

use async_broadcast::{broadcast, Receiver, Sender};
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
			circuit_breaker_timeout: Duration::from_secs(15),
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

#[derive(Debug)]
pub enum OBSCommand {
	SendRequest(serde_json::Value),
	Disconnect,
}

/// Core OBS WebSocket client - usable by both daemon and Axum server
pub struct ObsWebSocket {
	config: Arc<RwLock<ObsConfig>>,
	command_tx: Arc<RwLock<Option<tokio::sync::mpsc::Sender<OBSCommand>>>>,
	event_rx: Arc<RwLock<Option<Receiver<ObsEvent>>>>,
	task_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
	request_builder: ObsRequestBuilder,
}

impl ObsWebSocket {
	/// Create a new OBS WebSocket client
	#[must_use]
	fn new(config: ObsConfig) -> Self {
		Self {
			config: Arc::new(RwLock::new(config)),
			command_tx: Arc::new(RwLock::new(None)),
			event_rx: Arc::new(RwLock::new(None)),
			task_handle: Arc::new(RwLock::new(None)),
			request_builder: ObsRequestBuilder::new(),
		}
	}

	/// Connect with comprehensive polling using the new polling module
	async fn connect(&self, r: &[(ObsRequestType, PollingFrequency)]) -> Result<(), Box<dyn Error + Send + Sync>> {
		let config = self.config.read().await;
		let url = format!("ws://{}:{}", config.host, config.port);

		info!("Connecting to OBS WebSocket at {}", url);

		let (ws_stream, _) = connect_async(url).await?;
		let (sink, mut stream) = ws_stream.split();

		// TODO: should this be RWLock
		let s_a = Arc::new(tokio::sync::Mutex::new(sink));
		let s_p = s_a.clone();
		let s_m = s_a.clone();

		// Handle authentication
		{
			let mut s_g = s_a.lock().await;
			authenticate(&config.password, &mut s_g, &mut stream).await?;
			info!("Sucessfully connected!");
			fetch_init_state(&mut s_g).await?;
		}

		// Create channels for communication
		let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<OBSCommand>(10);
		let (mut event_tx, event_rx): (Sender<ObsEvent>, Receiver<ObsEvent>) = broadcast(3);

		event_tx.set_overflow(true);
		event_tx.set_await_active(false);

		*self.command_tx.write().await = Some(cmd_tx);
		*self.event_rx.write().await = Some(event_rx);

		// Start the comprehensive polling manager
		let polling_manager = ObsPollingManager::from_request_slice(r);
		let polling_task = tokio::spawn(async move {
			polling_manager.start_polling_loop(s_p, cmd_rx).await;
		});

		let message_task = tokio::spawn(async move {
			let mut consecutive_failures = 0;
			const MAX_CONSECUTIVE_FAILURES: usize = 10;
			let mut last_activity = Instant::now();
			const ACTIVITY_TIMEOUT: Duration = Duration::from_secs(120);

			let mut ping_i = tokio::time::interval(Duration::from_secs(30));
			ping_i.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

			let mut health = tokio::time::interval(Duration::from_secs(15));
			health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

			loop {
				tokio::select! {
					msg_r = stream.next() => {
						match msg_r {
							Some(msg) => {
								last_activity = Instant::now();

								match msg {
									Ok(TungsteniteMessage::Text(text)) => {
										match process_obs_message(text.to_string()).await {
											Ok(event) => {
												consecutive_failures = 0;
												match event_tx.broadcast(event).await {
													Ok(_) => {
														continue;
													}
													Err(e) => {
														error!("Failed to send event to channel: {}", e);
														break;
													}

												}
											}
											Err(e) => {
												consecutive_failures += 1;
												warn!("Failed to process OBS message (failure {}/{}): {}",
												consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
												);

												if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
													error!("Too many consecutive message processing failures. Breaking connection.");
													break;
												}
											}
										}
									}
									Ok(TungsteniteMessage::Ping(payload)) => {
										let mut s_g = s_m.lock().await;
										if let Err(e) = s_g.send(TungsteniteMessage::Pong(payload)).await {
											error!("Failed to send pong response: {}", e);
											break;
										}
									}
									Ok(TungsteniteMessage::Pong(_)) => {
									}
									Ok(TungsteniteMessage::Close(cl)) => {
										if let Some(fr) = cl {
											warn!("OBS sent close frame: code={}, reason={}", fr.code, fr.reason);
										} else {
											warn!("OBS sent close frame with no details");
										}
										break;
									}
									Ok(TungsteniteMessage::Binary(data)) => {
										warn!("Received unexpected binary message from OBS ({} bytes)", data.len());
									}
									Ok(tokio_tungstenite::tungstenite::Message::Frame(_)) => todo!(),
									Err(e) => {
										consecutive_failures += 1;
										error!(
											"WebSocket message error (failure {}/{}): {}",
											consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
										);

										if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
											error!("Too many consecutives WebSocket errors. Breaking connection");
											break;
										}
										continue
									}
								}
							}
							None => {
								error!("WebSocket stream ended unexpectedly - OBS connection lost");
								break;
							}
						}
					}

					_ = ping_i.tick() => {
						let mut s_g = s_m.lock().await;
						if let Err(e) = s_g.send(TungsteniteMessage::Ping(vec![].into())).await {
							error!("Failed to send keep-alive ping: {}", e);
							break;
						}
					}

					_ = health.tick() => {
						let el = last_activity.elapsed();
						if el > ACTIVITY_TIMEOUT {
							error!(
								"No activity from OBS for {:?} (timeouot: {:?}) - connection appears dead",
								el, ACTIVITY_TIMEOUT
							);
							break;
						} else {
							info!("Health check: last activity {:?} ago", el);
						}
					}
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

		*self.task_handle.write().await = Some(combined_task);

		info!("Connected to OBS WebSocket with comprehensive polling");
		Ok(())
	}

	/// Get the next event from OBS
	async fn next_event(&self) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
		let mut event_rx = self.event_rx.write().await;
		if let Some(ref mut rx) = *event_rx {
			match tokio::time::timeout(Duration::from_secs(30), rx.recv()).await {
				Ok(Ok(event)) => {
					return Ok(event);
				}
				Ok(Err(e)) => {
					error!("Event channel closed - OBS connection lost: {}", e);
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
	#[allow(dead_code)]
	async fn get_config(&self) -> ObsConfig {
		self.config.read().await.clone()
	}

	/// Update configuration
	async fn update_config(&self, config: ObsConfig) {
		*self.config.write().await = config;
	}

	/// Send a request to OBS
	async fn send_request(&mut self, request: serde_json::Value) -> Result<(), Box<dyn Error + Send + Sync>> {
		let tx = self.command_tx.read().await;
		if let Some(ref tx) = *tx {
			tx.try_send(OBSCommand::SendRequest(request))?;
			Ok(())
		} else {
			Err("Not connected".into())
		}
	}

	/// Start streaming
	async fn start_stream(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.start_stream();
		self.send_request(request).await
	}

	/// Stop streaming
	async fn stop_stream(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.stop_stream();
		self.send_request(request).await
	}

	/// Start recording
	async fn start_recording(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.start_recording();
		self.send_request(request).await
	}

	/// Stop recording
	async fn stop_recording(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.stop_recording();
		self.send_request(request).await
	}

	/// Switch to a specific scene
	async fn switch_scene(&mut self, scene_name: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.switch_scene(scene_name);
		self.send_request(request).await
	}

	/// Mute/unmute audio source
	async fn set_input_mute(&mut self, input_name: &str, muted: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.set_input_mute(input_name, muted);
		self.send_request(request).await
	}

	/// Set audio volume
	async fn set_input_volume(&mut self, input_name: &str, volume: f64) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.set_input_volume(input_name, volume);
		self.send_request(request).await
	}

	/// Toggle studio mode
	async fn toggle_studio_mode(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.toggle_studio_mode();
		self.send_request(request).await
	}

	/// Toggle virtual camera
	async fn toggle_virtual_camera(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.toggle_virtual_camera();
		self.send_request(request).await
	}

	/// Toggle replay buffer
	async fn toggle_replay_buffer(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let request = self.request_builder.toggle_replay_buffer();
		self.send_request(request).await
	}

	/// Check if connection is alive
	async fn is_connected(&self) -> bool {
		let has_command_tx = self.command_tx.read().await.is_some();
		let has_event_rx = self.event_rx.read().await.is_some();
		let task_active = {
			let handle = self.task_handle.read().await;
			handle.as_ref().map_or(false, |h| !h.is_finished())
		};

		has_command_tx && has_event_rx && task_active
	}

	/// Disconnect and cleanup
	async fn disconnect(&self) {
		if let Some(tx) = self.command_tx.read().await.as_ref() {
			let _ = tx.send(OBSCommand::Disconnect);
		}

		if let Some(handle) = self.task_handle.write().await.take() {
			handle.abort();
			let _ = handle.await;
		}

		*self.command_tx.write().await = None;
		*self.event_rx.write().await = None;
	}
}

/// Axum-specific wrapper that adds broadcasting functionality
pub struct ObsWebSocketWithBroadcast {
	inner: ObsWebSocket,
	broadcaster: Sender<ObsEvent>,
	receiver: Receiver<ObsEvent>,
}

impl ObsWebSocketWithBroadcast {
	/// Create new broadcaster-enabled OBS client
	pub fn new(config: ObsConfig) -> Self {
		let (mut sender, receiver) = broadcast(3);

		// Returns an error immediately if no active receivers exist
		sender.set_await_active(false);
		// Drops the oldest message instead of blocking.
		sender.set_overflow(true);

		Self {
			inner: ObsWebSocket::new(config),
			broadcaster: sender,
			receiver: receiver,
		}
	}

	/// Connect to OBS
	pub async fn connect(&self, r: &[(ObsRequestType, PollingFrequency)]) -> Result<(), Box<dyn Error + Send + Sync>> {
		self.inner.connect(r).await
	}

	/// Disconnect from OBS
	pub async fn disconnect(&self) {
		self.inner.disconnect().await;
	}

	/// Is Connected to OBS (true | false)
	pub async fn is_connected(&self) -> bool {
		self.inner.is_connected().await
	}

	/// Handle the next event and broadcast status updates
	pub async fn handle_next_event(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
		let event = self.inner.next_event().await?;

		if event.should_broadcast() {
			if let Err(e) = self.broadcaster.broadcast(event).await {
				error!("Failed to broadcast event: {}", e);
			}
		}

		Ok(())
	}

	/// Update configuration
	pub async fn update_config(&self, config: ObsConfig) {
		self.inner.update_config(config).await;
	}

	/// Get a new receiver for status updates
	pub fn subscribe(&self) -> Receiver<ObsEvent> {
		self.receiver.clone()
	}

	/// Create WebSocket handler for Axum
	pub fn websocket_handler(&self, ws: WebSocketUpgrade, State(obs_client): State<Arc<ObsWebSocketWithBroadcast>>) -> impl IntoResponse {
		let obs_receiver = obs_client.subscribe();

		ws.on_upgrade(move |socket| async move {
			handle_obs_socket(socket, obs_receiver).await;
		})
	}

	/// Start the event handling loop with broadcasting
	pub fn start(self: Arc<Self>, requests: Box<[(ObsRequestType, PollingFrequency)]>, retry_config: RetryConfig) -> BroadcastHandle {
		let c = self.clone();
		let b_h = self.broadcaster.clone();

		let handle = tokio::spawn(async move {
			c.connection_manager_loop(requests, retry_config).await;
		});

		BroadcastHandle {
			task_handle: handle,
			broadcaster: b_h,
		}
	}

	/// Main connection management loop
	async fn connection_manager_loop(&self, requests: Box<[(ObsRequestType, PollingFrequency)]>, retry_config: RetryConfig) {
		let mut retry_state = RetryState::Active {
			consecutive_failures: 0,
			current_delay: retry_config.initial_delay,
		};

		loop {
			match retry_state {
				RetryState::Active {
					consecutive_failures,
					current_delay,
				} => {
					retry_state = self.handle_connection_attempt(&requests, &retry_config, consecutive_failures, current_delay).await;
				}
				RetryState::CircuitOpen { opened_at } => {
					retry_state = Self::handle_circuit_breaker(opened_at, &retry_config).await;
				}
			}
		}
	}

	/// Handle a single connection attempt
	async fn handle_connection_attempt(
		&self,
		requests: &[(ObsRequestType, PollingFrequency)],
		retry_config: &RetryConfig,
		consecutive_failures: usize,
		current_delay: Duration,
	) -> RetryState {
		match self.connect(requests).await {
			Ok(()) => {
				info!("Connected to OBS WebSocket (attempt {} successful)", consecutive_failures + 1);

				// Run the event processing loop for this connection
				self.event_processing_loop().await;

				// Connection was lost during operation, try to reconnect immediately
				warn!("Connection lost during operation, attempting to reconnect...");
				self.disconnect().await;

				// Return to active state with reset failure count
				RetryState::Active {
					consecutive_failures: 0,
					current_delay: retry_config.initial_delay,
				}
			}
			Err(e) => Self::handle_connection_failure(e, consecutive_failures, current_delay, retry_config).await,
		}
	}

	/// Handle connection failure and determine next retry state
	async fn handle_connection_failure(error: Box<dyn Error + Send + Sync>, consecutive_failures: usize, current_delay: Duration, retry_config: &RetryConfig) -> RetryState {
		let new_failures = consecutive_failures + 1;
		error!("Failed to connect to OBS WebSocket (attempt {}): {}", new_failures, error);

		if new_failures >= retry_config.max_consecutive_failures {
			error!(
				"Max consecutive failures ({}) reached. Opening circuit breaker for {} seconds",
				retry_config.max_consecutive_failures,
				retry_config.circuit_breaker_timeout.as_secs()
			);

			RetryState::CircuitOpen { opened_at: Instant::now() }
		} else {
			// Calculate next delay with exponential backoff
			let next_delay = Duration::from_millis((current_delay.as_millis() as f64 * retry_config.backoff_multiplier) as u64).min(retry_config.max_delay);

			warn!(
				"Will retry in {} seconds (failure {}/{})",
				current_delay.as_secs(),
				new_failures,
				retry_config.max_consecutive_failures
			);

			tokio::time::sleep(current_delay).await;

			RetryState::Active {
				consecutive_failures: new_failures,
				current_delay: next_delay,
			}
		}
	}

	/// Handle circuit breaker logic
	async fn handle_circuit_breaker(opened_at: Instant, retry_config: &RetryConfig) -> RetryState {
		if opened_at.elapsed() >= retry_config.circuit_breaker_timeout {
			info!("Circuit breaker timeout elapsed, attempting to reconnect to OBS");
			RetryState::Active {
				consecutive_failures: 0,
				current_delay: retry_config.initial_delay,
			}
		} else {
			warn!("Circuit breaker open, waiting 30s before next retry!");
			tokio::time::sleep(Duration::from_secs(30)).await;
			RetryState::CircuitOpen { opened_at }
		}
	}

	/// Process events for an active connection
	async fn event_processing_loop(&self) {
		loop {
			// Check if still connected
			if !self.is_connected().await {
				warn!("OBS connection lost, exiting event processing loop");
				break;
			}

			// Try to get the next event
			match self.handle_next_event().await {
				Ok(()) => {
					continue;
				}
				Err(e) => {
					warn!("OBS WebSocket event error: {}", e);
					break;
				}
			}
		}
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
	mut obs_receiver: Receiver<ObsEvent>, // THIS WAS MISSING!
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
				Ok(Message::Text(_)) => {
					// Handle client commands here if needed
				}
				Ok(Message::Close(frame)) => {
					warn!("WebSocket client requested close: {:?}", frame);
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
	warn!("WebSocket connection closed");
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
	broadcaster: Sender<ObsEvent>,
}

impl BroadcastHandle {
	/// Get a new receiver for status updates
	pub fn subscribe(&self) -> Receiver<ObsEvent> {
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

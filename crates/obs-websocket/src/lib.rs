// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It follows the singleton pattern and can be easily integrated with any Axum server.

mod auth;
mod messages;

use auth::authenticate;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{
	sink::SinkExt,
	stream::{SplitSink, SplitStream, StreamExt},
};
use messages::{fetch_init_state, process_obs_message};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream};
use tracing::{debug, error, info, warn};

static INSTANCE: Lazy<ObsWebSocketClient> = Lazy::new(|| ObsWebSocketClient::new());

/// Main entry point for the library
#[must_use]
pub fn client() -> &'static ObsWebSocketClient {
	&INSTANCE
}

/// OBS WebSocket client that manages the connection and state
pub struct ObsWebSocketClient {
	config: Arc<RwLock<ObsConfig>>,
	status: Arc<RwLock<ObsStatus>>,
	broadcaster: async_broadcast::Sender<ObsStatus>,
	_receiver: async_broadcast::Receiver<ObsStatus>,
}

/// Configuration for the OBS WebSocket connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsConfig {
	pub host: String,
	pub port: u16,
	pub password: String,
}

/// Current status of OBS
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
			host: "localhost".to_string(),
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

impl ObsWebSocketClient {
	/// Create a new OBS WebSocket client
	fn new() -> Self {
		let (sender, receiver) = async_broadcast::broadcast(100);
		Self {
			config: Arc::new(RwLock::new(ObsConfig::default())),
			status: Arc::new(RwLock::new(ObsStatus::default())),
			broadcaster: sender,
			_receiver: receiver,
		}
	}

	/// Start the OBS WebSocket client
	pub fn start(&self) {
		// Clone references for the background task
		let config = Arc::clone(&self.config);
		let status = Arc::clone(&self.status);
		let broadcaster = self.broadcaster.clone();

		// Start the OBS WebSocket client in a background task
		tokio::spawn(async move {
			obs_websocket_client(config, status, broadcaster).await;
		});
	}

	/// Get the current OBS status
	pub async fn get_status(&self) -> ObsStatus {
		self.status.read().await.clone()
	}

	/// Get the current OBS configuration
	pub async fn get_config(&self) -> ObsConfig {
		self.config.read().await.clone()
	}

	/// Update the OBS configuration
	pub async fn update_config(&self, config: ObsConfig) {
		*self.config.write().await = config;
		// The reconnection will happen automatically in the background task
	}

	/// Create a WebSocket handler for Axum
	pub fn websocket_handler(&self, ws: WebSocketUpgrade) -> impl IntoResponse {
		let broadcaster = self.broadcaster.clone();
		let status = Arc::clone(&self.status);

		ws.on_upgrade(move |socket| async move {
			handle_socket(socket, status, broadcaster).await;
		})
	}

	/// Get a new receiver for status updates
	pub fn subscribe(&self) -> async_broadcast::Receiver<ObsStatus> {
		self.broadcaster.new_receiver()
	}
}

// Private implementation
async fn handle_socket(socket: WebSocket, status: Arc<RwLock<ObsStatus>>, broadcaster: async_broadcast::Sender<ObsStatus>) {
	let status_snapshot = status.read().await.clone();
	let (mut sender, mut receiver) = socket.split();

	match serde_json::to_string(&status_snapshot) {
		Ok(res) => {
			if let Err(e) = sender.send(Message::Text(res)).await {
				error!("Error sending initial status: {}", e);
				return;
			}
			info!("sent initial status!");
		}
		Err(e) => {
			error!("Serialization failed with error code: {e}");
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
						info!("sent status update?!");
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

	// Wait for both tasks to complete
	let _ = tokio::join!(send_task, recv_task);
}

// Main OBS WebSocket client loop
async fn obs_websocket_client(config: Arc<RwLock<ObsConfig>>, status: Arc<RwLock<ObsStatus>>, broadcaster: async_broadcast::Sender<ObsStatus>) {
	info!("Started obs websocket...");
	let mut reconnect_interval = interval(Duration::from_secs(5));

	loop {
		reconnect_interval.tick().await;

		let config_snapshot = config.read().await.clone();
		let url = format!("ws://{}:{}", config_snapshot.host, config_snapshot.port);

		info!("Connecting to OBS WebSocket at {}", url);

		match connect_async(url).await {
			Ok((ws_stream, _)) => {
				info!("Connected to OBS WebSocket");
				let (sink, stream) = ws_stream.split();

				// Handle the OBS WebSocket connection
				if let Err(e) = handle_obs_connection(sink, stream, config_snapshot.password, status.clone(), broadcaster.clone()).await {
					error!("OBS WebSocket error: {}", e);
				}
			}
			Err(e) => {
				error!("Failed to connect to OBS WebSocket: {}", e);
			}
		}

		warn!("OBS connection lost, reconnecting in 5 seconds...");
	}
}

// Handle active OBS WebSocket connection
async fn handle_obs_connection(
	mut sink: SplitSink<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>,
	mut stream: SplitStream<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>>,
	password: String,
	status: Arc<RwLock<ObsStatus>>,
	broadcaster: async_broadcast::Sender<ObsStatus>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
	// Wait for hello message

	// Handle authentication
	authenticate(&password, &mut sink, &mut stream).await?;

	fetch_init_state(&mut sink).await?;

	// Set up status polling
	let (tx, mut rx) = mpsc::channel(10);

	// Clone sink for the polling task
	let status_sink = sink.reunite(stream)?;
	let (mut sink, mut stream) = status_sink.split();

	// Spawn status polling task
	tokio::spawn(async move {
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
			debug!("Polling: sending stream status request: {}", status_req);
			if let Err(e) = sink.send(TungsteniteMessage::Text(status_req.to_string().into())).await {
				error!("Failed to send stream status request: {}", e);
				break;
			}

			// Request recording status
			// TODO: Do I care about this?
			let recording_req = json!({
				"op": 6,
				"d": {
					"requestType": "GetRecordStatus",
					"requestId": format!("recording-{}", request_id)
				}
			});
			debug!("Polling: sending recording status request: {}", recording_req);
			if let Err(e) = sink.send(TungsteniteMessage::Text(recording_req.to_string().into())).await {
				error!("Failed to send recording status request: {}", e);
				break;
			}

			// Flush the sink to send all queued messages
			if let Err(e) = sink.flush().await {
				error!("Failed to flush sink: {e}");
				break;
			}

			// Send keepalive to channel to indicate task is alive
			if tx.send(()).await.is_err() {
				error!("Boyo is dead!");
				break;
			}
		}
	});

	// Main message processing loop
	loop {
		tokio::select! {
			// Process incoming OBS messages
			message = stream.next() => {
				match message {
					Some(Ok(TungsteniteMessage::Text(text))) => {
						process_obs_message(text.to_string(), status.clone(), broadcaster.clone()).await?;
					}
					Some(Ok(other)) => {
						debug!("Received non-text WebSocket message: {:?}", other);
					}
					Some(Err(e)) => {
						error!("WebSocket error: {}", e);
						break;
					}
					None => {
						info!("OBS WebSocket connection closed");
						break;
					}
				}
			}

			// Check if polling task is still alive
			_ = rx.recv() => {
				// Polling task is still running
			}
		}
	}

	warn!("we exited the main loop why?!");
	Ok(())
}

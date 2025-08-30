use super::*;
use crate::*;
use futures_util::{
	future,
	sink::SinkExt,
	stream::{SplitSink, SplitStream, StreamExt},
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::{
	net::TcpStream,
	sync::{mpsc, Mutex},
	task::JoinHandle,
};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream, WebSocketStream};

/// Connection-specific error types
#[derive(Error, Debug)]
pub enum ConnectionError {
	#[error("Failed to connect to WebSocket: {0}")]
	WebSocketConnection(#[from] tokio_tungstenite::tungstenite::Error),

	#[error("Authentication failed: {0}")]
	Authentication(String),

	#[error("Connection initialization failed: {0}")]
	Initialization(String),

	#[error("State transition error: {0}")]
	State(#[from] StateError),

	#[error("Connection is not healthy")]
	Unhealthy,

	#[error("Connection timeout after {timeout}s")]
	Timeout { timeout: u64 },

	#[error("Connection lost unexpectedly")]
	Lost,

	#[error("Invalid configuration: {0}")]
	Config(String),

	#[error("Internal communication error: {0}")]
	Communication(String),
}

/// Connection manager that handles the WebSocket connection lifecycle
pub struct ConnectionManager {
	state_handle: StateHandle,
	msg_handler: MessageHandler,
}

impl ConnectionManager {
	pub fn new(state_handle: StateHandle) -> Self {
		Self {
			state_handle,
			msg_handler: MessageHandler::new(),
		}
	}

	/// Establish connection and set up communication channels
	pub async fn establish_connection(&self, requests: &[(ObsRequestType, PollingFrequency)]) -> Result<(), ConnectionError> {
		// Transition to connecting state
		self.state_handle.transition_to_connecting().await?;

		// Get config from state
		let config = self.state_handle.config().await?;
		let url = format!("ws://{}:{}", config.host, config.port);

		// Attempt WebSocket connection
		let (ws_stream, _) = match connect_async(&url).await {
			Ok(result) => result,
			Err(e) => {
				let error_msg = e.to_string();
				self.state_handle.transition_to_failed(error_msg.clone()).await?;
				return Err(ConnectionError::WebSocketConnection(e));
			}
		};

		let (sink, mut stream) = ws_stream.split();
		let sink = Arc::new(Mutex::new(sink));

		// Authenticate
		if let Err(e) = self.authenticate_and_init(&config, &sink, &mut stream).await {
			let error_msg = e.to_string();
			self.state_handle.transition_to_failed(error_msg.clone()).await?;
			return Err(ConnectionError::Authentication(error_msg));
		}

		// Set up channels
		let (cmd_tx, cmd_rx) = mpsc::channel(10);
		let (mut event_tx, event_rx) = async_broadcast::broadcast(3);
		event_tx.set_overflow(true);
		event_tx.set_await_active(false); // Don't wait when broadcasting

		// Update state with communication channels
		// Store both sender and receiver
		self.state_handle.set_command_sender(cmd_tx).await?;
		self.state_handle.set_event_sender(event_tx.clone()).await?;
		self.state_handle.set_event_receiver(event_rx).await?;

		// Start connection tasks
		let connection_handle = self.start_connection_tasks(sink, stream, cmd_rx, event_tx, requests).await;
		self.state_handle.set_connection_handle(connection_handle).await?;

		// Transition to connected state
		self.state_handle.transition_to_connected().await?;

		Ok(())
	}

	/// Clean up connection resources
	pub async fn cleanup_connection(&self) -> Result<(), ConnectionError> {
		// Transition to disconnecting state if we're connected
		if self.state_handle.is_connected().await? {
			let _ = self.state_handle.transition_to_disconnecting().await;
		}

		// Clean up resources
		if let Ok(Some(handle)) = self.state_handle.take_connection_handle().await {
			handle.abort();
			let _ = handle.await;
		}

		let _ = self.state_handle.take_command_sender().await;
		let _ = self.state_handle.take_event_receiver().await;

		// Transition to disconnected state
		self.state_handle.transition_to_disconnected().await?;

		Ok(())
	}

	/// Disconnect gracefully
	pub async fn disconnect(&self) -> Result<(), ConnectionError> {
		if !self.state_handle.is_connected().await? {
			return Ok(());
		}

		// Send disconnect command if we have a command sender
		if let Ok(Some(sender)) = self.state_handle.take_command_sender().await {
			let _ = sender.try_send(InternalCommand::Disconnect);
			// Put the sender back
			let _ = self.state_handle.set_command_sender(sender).await;
		}

		// Wait a bit for graceful shutdown
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Clean up
		self.cleanup_connection().await
	}

	/// Check if the connection is healthy
	pub async fn is_healthy(&self) -> Result<bool, ConnectionError> {
		let connected = self.state_handle.is_connected().await?;
		let can_execute = self.state_handle.can_execute_commands().await?;
		Ok(connected && can_execute)
	}

	/// Get connection info for monitoring
	pub async fn connection_info(&self) -> Result<ConnectionInfo, ConnectionError> {
		let state = self.state_handle.connection_state().await?;
		let config = self.state_handle.config().await?;
		let healthy = self.is_healthy().await?;

		Ok(ConnectionInfo {
			state,
			host: config.host,
			port: config.port,
			healthy,
		})
	}

	async fn authenticate_and_init(
		&self,
		config: &ObsConfig,
		sink: &Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>>>,
		stream: &mut SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
	) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		let mut sink_guard = sink.lock().await;

		// Authenticate
		authenticate(&config.password, &mut sink_guard, stream).await?;

		// Fetch initial state
		self.msg_handler.initialize(&mut sink_guard).await?;

		Ok(())
	}

	async fn start_connection_tasks(
		&self,
		sink: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>>>,
		stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
		cmd_rx: mpsc::Receiver<InternalCommand>,
		event_tx: async_broadcast::Sender<ObsEvent>,
		requests: &[(ObsRequestType, PollingFrequency)],
	) -> JoinHandle<()> {
		let sink_for_polling = sink.clone();
		let message_processor = self.msg_handler.processor();
		let state_handle = self.state_handle.clone();
		let cmd_exec = CommandExecutor::new(self.state_handle.clone());
		let polling_manager = ObsPollingManager::from_request_slice(requests, cmd_exec);

		let polling_task = tokio::spawn(async move {
			let _ = polling_manager.start_polling_loop(sink_for_polling, cmd_rx).await;
		});

		let message_task = tokio::spawn(async move {
			message_processing_loop(stream, sink, event_tx, state_handle, message_processor).await;
		});

		tokio::spawn(async move {
			tokio::select! {
				_ = polling_task => {
					tracing::error!("Polling task ended unexpectedly");
				}
				_ = message_task => {
					tracing::error!("Message processing task ended unexpectedly");
				}
			}
		})
	}
}

async fn message_processing_loop(
	mut stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
	sink: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>>>,
	event_tx: async_broadcast::Sender<ObsEvent>,
	state_handle: StateHandle,
	message_processor: MessageProcessor,
) {
	let mut last_activity = Instant::now();
	let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
	ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

	loop {
		tokio::select! {
			msg = stream.next() => {
				match msg {
					Some(Ok(TungsteniteMessage::Text(text))) => {
						last_activity = Instant::now();
						// TODO: need to update the message mod to work with expected type directly
						 if let Ok(event) = message_processor.process_message(text.to_string()).await {
							let _ = event_tx.broadcast(event).await;
						}
					}
					Some(Ok(TungsteniteMessage::Ping(payload))) => {
						last_activity = Instant::now();
						let mut sink_guard = sink.lock().await;
						let _ = sink_guard.send(TungsteniteMessage::Pong(payload)).await;
					}
					Some(Ok(TungsteniteMessage::Pong(_))) => {
						last_activity = Instant::now();
					}
					Some(Ok(TungsteniteMessage::Close(_))) => {
						tracing::info!("WebSocket close frame received");
						break;
					}
					Some(Err(e)) => {
						tracing::error!("WebSocket error: {}", e);
						break;
					}
					None => {
						tracing::info!("WebSocket stream ended");
						break;
					}
					_ => {} // Handle other message types if needed
				}
			}
			_ = ping_interval.tick() => {
				if last_activity.elapsed() > Duration::from_secs(120) {
					tracing::error!("Connection appears dead (no activity for 120s), breaking");
					break;
				}

				let mut sink_guard = sink.lock().await;
				if sink_guard.send(TungsteniteMessage::Ping(vec![].into())).await.is_err() {
					tracing::error!("Failed to send ping, connection likely dead");
					break;
				}
			}
		}
	}

	// Connection ended, transition state to disconnected
	tracing::info!("Message processing loop ended, transitioning to disconnected");
	let _ = state_handle.transition_to_disconnected().await;
}

/// Connection information for monitoring
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
	pub state: ConnectionState,
	pub host: String,
	pub port: u16,
	pub healthy: bool,
}

impl ConnectionInfo {
	pub fn is_active(&self) -> bool {
		matches!(self.state, ConnectionState::Connected { .. })
	}

	pub fn uptime(&self) -> Option<Duration> {
		match &self.state {
			ConnectionState::Connected { connected_at } => Some(connected_at.elapsed()),
			_ => None,
		}
	}
}

/// High-level connection API that combines all connection operations
pub struct ObsConnection {
	connection_manager: ConnectionManager,
	event_handler: EventHandler,
	command_executor: CommandExecutor,
}

impl ObsConnection {
	pub fn new(state_handle: StateHandle) -> Self {
		Self {
			connection_manager: ConnectionManager::new(state_handle.clone()),
			event_handler: EventHandler::new(state_handle.clone()),
			command_executor: CommandExecutor::new(state_handle),
		}
	}

	/// Connect to OBS with polling requests
	pub async fn connect(&self, requests: &[(ObsRequestType, PollingFrequency)]) -> Result<(), ConnectionError> {
		self.connection_manager.establish_connection(requests).await
	}

	/// Disconnect from OBS
	pub async fn disconnect(&self) -> Result<(), ConnectionError> {
		self.connection_manager.disconnect().await
	}

	/// Execute a command
	pub async fn execute_command(&self, command: ObsCommand) -> Result<(), ConnectionError> {
		self.command_executor.execute(command).await.map_err(|e| ConnectionError::Communication(e.to_string()))
	}

	/// Get the next event
	pub async fn next_event(&self) -> Result<ObsEvent, ConnectionError> {
		self.event_handler.next_event().await.map_err(|e| ConnectionError::Communication(e.to_string()))
	}

	/// Stream events with a handler
	pub async fn stream_events<F>(&self, handler: F) -> Result<(), ConnectionError>
	where
		F: FnMut(ObsEvent) -> future::BoxFuture<'static, ()>,
	{
		self.event_handler.stream_events(handler).await.map_err(|e| ConnectionError::Communication(e.to_string()))
	}

	/// Get connection info
	pub async fn connection_info(&self) -> Result<ConnectionInfo, ConnectionError> {
		self.connection_manager.connection_info().await
	}

	/// Check if healthy
	pub async fn is_healthy(&self) -> Result<bool, ConnectionError> {
		self.connection_manager.is_healthy().await
	}
}

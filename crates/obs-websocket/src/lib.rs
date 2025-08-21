// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It separates core OBS logic from context-specific concerns (broadcasting, etc.)

use async_broadcast::Receiver;
use std::sync::Arc;
use thiserror::Error;

mod auth;
mod config;
mod core;
mod messages;
mod polling;

use auth::authenticate;
use config::ObsConfig;
use core::*;
use messages::*;
use polling::{ObsPollingManager, ObsRequestBuilder, ObsRequestType, PollingFrequency};

/// Errors for obs-websocket crate
#[derive(Debug, Error)]
pub enum ObsWebsocketError {
	#[error("Connection error: {0}")]
	ObsConnection(#[from] ConnectionError),

	#[error("State error: {0}")]
	ObsState(#[from] StateError),

	#[error("Broadcast error: {0}")]
	ObsBroadcastError(#[from] BroadcastError),

	#[error("Command execution failed: {0}")]
	CommandFailed(String),
}

/// Core OBS WebSocket manager with state machine guarantees
pub struct ObsWebSocketManager {
	obs_connection: ObsConnection,
	retry_policy: RetryPolicy,
	_state_actor_handle: tokio::task::JoinHandle<()>, // Keep actor alive
}

impl ObsWebSocketManager {
	pub fn new(config: ObsConfig, retry_config: RetryConfig) -> Self {
		// Create state actor and handle
		let (state_actor, state_handle) = StateActor::new(config);

		// Spawn the state actor
		let state_actor_handle = tokio::spawn(async move {
			state_actor.run().await;
		});

		Self {
			obs_connection: ObsConnection::new(state_handle),
			retry_policy: RetryPolicy::new(retry_config),
			_state_actor_handle: state_actor_handle,
		}
	}

	/// Connect with polling configuration
	pub async fn connect(&self, requests: &[(ObsRequestType, PollingFrequency)]) -> Result<(), ObsWebsocketError> {
		self.obs_connection.connect(requests).await?;
		Ok(())
	}

	/// Disconnect and cleanup
	pub async fn disconnect(&self) -> Result<(), ObsWebsocketError> {
		self.obs_connection.disconnect().await?;
		Ok(())
	}

	/// Get current connection info
	pub async fn connection_info(&self) -> Result<ConnectionInfo, ObsWebsocketError> {
		let conn_info = self.obs_connection.connection_info().await?;
		Ok(conn_info)
	}

	/// Get current state from the state actor
	pub async fn current_state(&self) -> Result<ConnectionState, ObsWebsocketError> {
		// Get state directly from the connection's state handle
		let info = self.obs_connection.connection_info().await?;
		Ok(info.state)
	}

	/// Check if connection is healthy
	pub async fn is_healthy(&self) -> Result<bool, ObsWebsocketError> {
		let result = self.obs_connection.is_healthy().await?;
		Ok(result)
	}

	/// Execute a command (validates state before execution)
	pub async fn execute_command(&self, command: ObsCommand) -> Result<(), ObsWebsocketError> {
		let _ = self.obs_connection.execute_command(command).await;
		Ok(())
	}

	/// Get next event
	pub async fn next_event(&self) -> Result<ObsEvent, ObsWebsocketError> {
		let event = self.obs_connection.next_event().await?;
		Ok(event)
	}

	/// Stream events with a handler function
	pub async fn stream_events<F>(&self, handler: F) -> Result<(), ObsWebsocketError>
	where
		F: FnMut(ObsEvent) -> futures_util::future::BoxFuture<'static, ()>,
	{
		self.obs_connection.stream_events(handler).await?;
		Ok(())
	}
}

/// Broadcast-enabled manager for server applications
pub struct ObsBroadcastManager {
	manager: ObsWebSocketManager,
	broadcaster: EventBroadcaster,
}

impl ObsBroadcastManager {
	pub fn new(config: ObsConfig, retry_config: RetryConfig) -> Self {
		Self {
			manager: ObsWebSocketManager::new(config, retry_config),
			broadcaster: EventBroadcaster::new(),
		}
	}

	/// Start the management loop with automatic reconnection
	pub fn start(self: Arc<Self>, requests: Box<[(ObsRequestType, PollingFrequency)]>) -> BroadcastHandle {
		let task_handle = tokio::spawn(async move {
			loop {
				// Attempt connection with retry logic
				match self.manager.connect(&requests).await {
					Ok(()) => {
						tracing::info!("Connected to OBS WebSocket");

						// Start event streaming to broadcaster
						let event_result = self
							.manager
							.stream_events(|event| {
								let broadcaster = self.broadcaster.sender.clone();
								Box::pin(async move {
									if event.should_broadcast() {
										if let Err(e) = broadcaster.broadcast(event).await {
											tracing::warn!("Failed to broadcast event: {}", e);
										}
									}
								})
							})
							.await;

						if let Err(e) = event_result {
							tracing::error!("Event streaming error: {}", e);
						}
					}
					Err(e) => {
						tracing::error!("Connection failed: {}", e);
					}
				}

				// Clean disconnect before retry
				if let Err(e) = self.manager.disconnect().await {
					tracing::warn!("Error during disconnect: {}", e);
				}

				// Retry delay
				tokio::time::sleep(std::time::Duration::from_secs(5)).await;
			}
		});

		BroadcastHandle::new(task_handle)
	}

	/// Subscribe to events
	pub fn subscribe(&self) -> Receiver<ObsEvent> {
		self.broadcaster.subscribe()
	}

	/// Get WebSocket handler for Axum
	pub fn websocket_handler(&self) -> WebSocketHandler {
		WebSocketHandler::new(self.broadcaster.subscribe())
	}

	/// Delegate command execution
	pub async fn execute_command(&self, command: ObsCommand) -> Result<(), ObsWebsocketError> {
		self.manager.execute_command(command).await
	}

	/// Get connection info
	pub async fn connection_info(&self) -> Result<ConnectionInfo, ObsWebsocketError> {
		self.manager.connection_info().await
	}

	/// Check if healthy
	pub async fn is_healthy(&self) -> Result<bool, ObsWebsocketError> {
		self.manager.is_healthy().await
	}

	/// Disconnect
	pub async fn disconnect(&self) -> Result<(), ObsWebsocketError> {
		self.manager.disconnect().await
	}

	/// Get current connection state
	pub async fn current_state(&self) -> Result<ConnectionState, ObsWebsocketError> {
		self.manager.current_state().await
	}
}

// Convenience constructors
pub fn create_obs_manager(config: ObsConfig) -> ObsWebSocketManager {
	ObsWebSocketManager::new(config, RetryConfig::default())
}

pub fn create_obs_broadcast_manager(config: ObsConfig) -> ObsBroadcastManager {
	ObsBroadcastManager::new(config, RetryConfig::default())
}

/// Handle for managing broadcast operations
pub struct BroadcastHandle {
	task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl BroadcastHandle {
	fn new(task_handle: tokio::task::JoinHandle<()>) -> Self {
		Self { task_handle: Some(task_handle) }
	}

	/// Stop the broadcast manager
	pub async fn stop(&mut self) -> Result<(), ObsWebsocketError> {
		if let Some(handle) = self.task_handle.take() {
			// abort first to stop it quickly
			handle.abort();

			match handle.await {
				Ok(()) => Ok(()),
				Err(e) if e.is_cancelled() => Ok(()),
				Err(e) => Err(ObsWebsocketError::CommandFailed(format!("Failed to stop broadcast handle: {}", e))),
			}
		} else {
			// already stopped
			Ok(())
		}
	}

	/// Check if the broadcast task is still running
	pub fn is_running(&self) -> bool {
		self.task_handle.as_ref().map(|h| !h.is_finished()).unwrap_or(false)
	}
}

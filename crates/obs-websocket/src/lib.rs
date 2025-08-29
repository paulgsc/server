// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It separates core OBS logic from context-specific concerns (broadcasting, etc.)

use thiserror::Error;

mod auth;
mod config;
pub mod core;
mod messages;
mod polling;

use auth::authenticate;
pub use config::ObsConfig;
pub use core::*;
pub use messages::{MessageHandler, MessageProcessor, ObsEvent};
use polling::{ObsPollingManager, ObsRequestBuilder};
pub use polling::{ObsRequestType, PollingConfig, PollingFrequency};

/// Errors for obs-websocket crate
#[derive(Debug, Error)]
pub enum ObsWebsocketError {
	#[error("Connection error: {0}")]
	ObsConnection(#[from] ConnectionError),

	#[error("State error: {0}")]
	ObsState(#[from] StateError),

	#[error("Command execution failed: {0}")]
	CommandFailed(String),
}

/// Core OBS WebSocket manager with state machine guarantees
pub struct ObsWebSocketManager {
	obs_connection: ObsConnection,
	#[allow(dead_code)]
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

// Convenience constructors
pub fn create_obs_manager(config: ObsConfig) -> ObsWebSocketManager {
	ObsWebSocketManager::new(config, RetryConfig::default())
}

// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It separates core OBS logic from context-specific concerns (broadcasting, etc.)

#[cfg(feature = "websocket")]
use thiserror::Error;

// Always available - types only
pub mod types;
pub use types::{ObsEvent, UnknownEventData, YouTubePrivacy};

// Feature-gated modules
#[cfg(feature = "websocket")]
mod auth;
#[cfg(feature = "websocket")]
mod config;
#[cfg(feature = "websocket")]
pub mod core;
#[cfg(feature = "websocket")]
pub mod messages;
#[cfg(feature = "websocket")]
mod polling;

// Feature-gated exports
#[cfg(feature = "websocket")]
use auth::authenticate;
#[cfg(feature = "websocket")]
pub use config::ObsConfig;
#[cfg(feature = "websocket")]
pub use core::*;
#[cfg(feature = "websocket")]
pub use messages::{MessageHandler, MessageProcessor};
#[cfg(feature = "websocket")]
use polling::{ObsPollingManager, ObsRequestBuilder};
#[cfg(feature = "websocket")]
pub use polling::{PollingConfig, PollingFrequency};

/// Errors for obs-websocket crate
#[cfg(feature = "websocket")]
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
#[cfg(feature = "websocket")]
pub struct ObsWebSocketManager {
	obs_connection: ObsConnection,
	#[allow(dead_code)]
	retry_policy: RetryPolicy,
	_state_actor_handle: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "websocket")]
impl ObsWebSocketManager {
	pub fn new(config: ObsConfig, retry_config: RetryConfig) -> Self {
		let (state_actor, state_handle) = StateActor::new(config);
		let state_actor_handle = tokio::spawn(async move {
			state_actor.run().await;
		});
		Self {
			obs_connection: ObsConnection::new(state_handle),
			retry_policy: RetryPolicy::new(retry_config),
			_state_actor_handle: state_actor_handle,
		}
	}

	pub async fn connect(&self, config: PollingConfig) -> Result<(), ObsWebsocketError> {
		self.obs_connection.connect(config).await?;
		Ok(())
	}

	pub async fn disconnect(&self) -> Result<(), ObsWebsocketError> {
		self.obs_connection.disconnect().await?;
		Ok(())
	}

	pub async fn connection_info(&self) -> Result<ConnectionInfo, ObsWebsocketError> {
		let conn_info = self.obs_connection.connection_info().await?;
		Ok(conn_info)
	}

	pub async fn current_state(&self) -> Result<ConnectionState, ObsWebsocketError> {
		let info = self.obs_connection.connection_info().await?;
		Ok(info.state)
	}

	pub async fn is_healthy(&self) -> Result<bool, ObsWebsocketError> {
		let result = self.obs_connection.is_healthy().await?;
		Ok(result)
	}

	pub async fn execute_command(&self, command: ObsCommand) -> Result<(), ObsWebsocketError> {
		let _ = self.obs_connection.execute_command(command).await;
		Ok(())
	}

	pub async fn next_event(&self) -> Result<ObsEvent, ObsWebsocketError> {
		let event = self.obs_connection.next_event().await?;
		Ok(event)
	}

	pub async fn stream_events<F>(&self, handler: F) -> Result<(), ObsWebsocketError>
	where
		F: FnMut(ObsEvent) -> futures_util::future::BoxFuture<'static, ()>,
	{
		self.obs_connection.stream_events(handler).await?;
		Ok(())
	}
}

#[cfg(feature = "websocket")]
pub fn create_obs_manager(config: ObsConfig) -> ObsWebSocketManager {
	ObsWebSocketManager::new(config, RetryConfig::default())
}

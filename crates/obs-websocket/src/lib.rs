#![allow(clippy::wildcard_imports)]
#![allow(clippy::enum_glob_use)]
#![allow(clippy::unused_async)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::needless_continue)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::type_complexity)]
#![allow(clippy::disallowed_methods)]
#![allow(clippy::disallowed_macros)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::future_not_send)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::result_large_err)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::single_match)]
#![allow(clippy::single_match_else)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::match_wildcard_for_single_variants)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unused_self)]
#![allow(clippy::too_long_first_doc_paragraph)]
#![allow(clippy::ignored_unit_patterns)]
// axum-obs-websocket Library
//
// This library provides a clean interface to interact with OBS via WebSocket.
// It separates core OBS logic from context-specific concerns (broadcasting, etc.)

#[cfg(feature = "websocket")]
use thiserror::Error;

// Always available - types only
pub mod types;
pub use types::{ObsCommand, ObsEvent, UnknownEventData, YouTubePrivacy};

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

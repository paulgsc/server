//! OBS control for this workspace, built on [`obws`].
//!
//! [`types`] is always available and holds the wire types other crates send
//! over NATS. The `websocket` feature adds [`ObsWebSocketManager`], which
//! connects to OBS, publishes its events and status snapshots as [`ObsEvent`]s,
//! and runs [`ObsCommand`]s against it.

pub mod types;
pub use types::{MediaAction, ObsCommand, ObsEvent, StreamKey, StudioSnapshot, UnknownEventData};

#[cfg(feature = "websocket")]
mod commands;
#[cfg(feature = "websocket")]
mod config;
#[cfg(feature = "websocket")]
mod events;
#[cfg(feature = "websocket")]
mod manager;
#[cfg(feature = "websocket")]
mod polling;
#[cfg(feature = "websocket")]
mod studio;

#[cfg(feature = "websocket")]
pub use config::{ObsConfig, RetryConfig};
#[cfg(feature = "websocket")]
pub use manager::{CommandError, ConnectionState, ObsWebSocketManager, ObsWebsocketError, COMMAND_TIMEOUT};
#[cfg(feature = "websocket")]
pub use polling::{PollTarget, PollingConfig};

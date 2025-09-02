use crate::core::{CommandExecutor, InternalCommand, StateError};
use futures_util::sink::SinkExt;
use futures_util::stream::SplitSink;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, WebSocketStream};
use tracing::error;

pub type WsSink = SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, TungsteniteMessage>;
pub type SharedSink = Arc<tokio::sync::Mutex<WsSink>>;

pub mod config;
mod error;
mod manager;
mod request_builder;

pub use config::*;
pub use error::*;
pub use manager::*;
pub use request_builder::*;

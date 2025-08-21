use crate::core::{CommandExecutor, InternalCommand, StateError};
use futures_util::sink::SinkExt;
use futures_util::stream::SplitSink;
use serde_json::json;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, WebSocketStream};
use tracing::{error, info, instrument};

pub type WsSink = SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, TungsteniteMessage>;
pub type SharedSink = Arc<tokio::sync::Mutex<WsSink>>;

mod config;
mod error;
mod manager;
mod request_builder;
mod requests;

pub(crate) use config::*;
pub(crate) use error::*;
pub(crate) use manager::*;
pub(crate) use request_builder::*;
pub(crate) use requests::*;

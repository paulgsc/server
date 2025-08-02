// use async_trait::async_trait;
// use bytes::Bytes;
// use futures::stream::{SplitSink, SplitStream};
// use futures::{SinkExt, Stream, StreamExt};
// use prometheus::{register_counter, register_gauge, register_histogram, register_int_gauge, Counter, Gauge, Histogram, IntGauge};
// use std::collections::{HashMap, VecDeque};
// use std::net::SocketAddr;
// use std::pin::Pin;
// use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
// use std::sync::Arc;
// use std::time::{Duration, Instant};
// use tokio::net::TcpStream;
// use tokio::sync::{broadcast, mpsc, oneshot};
// use tokio_tungstenite::{
// 	tungstenite::{protocol::CloseFrame, Message},
// 	MaybeTlsStream, WebSocketStream,
// };
// use tracing::{debug, error, info, span, warn, Instrument, Level};
//

pub mod connection;
mod message;
mod protocol;
mod transport;

pub use transport::error::TransportError;

// // Re-export main types for convenience
// pub use transport_actor::TransportActor;
// pub use tungstenite_transport::TungsteniteTransport;
//
// // Module organization
// mod transport_actor {
// 	pub use super::TransportActor;
// }
//
// mod tungstenite_transport {
// 	pub use super::TungsteniteTransport;
// }
//
// /// Factory function to create a new transport actor with tungstenite
// pub fn create_websocket_transport(config: TransportConfig) -> (TransportActor<TungsteniteTransport>, mpsc::Sender<TransportCommand>, broadcast::Receiver<TransportEvent>) {
// 	let (command_tx, command_rx) = mpsc::channel(32);
// 	let (event_tx, event_rx) = broadcast::channel(256);
//
// 	let connection_id = ConnectionId::new();
// 	let metrics = Arc::new(TransportMetrics::new(&connection_id.to_string()).unwrap());
//
// 	let transport = TungsteniteTransport::new();
// 	let actor = TransportActor::new(config, transport, command_rx, event_tx, metrics);
//
// 	(actor, command_tx, event_rx)
// }

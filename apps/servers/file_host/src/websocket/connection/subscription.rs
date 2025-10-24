use super::errors::ConnectionError;
use crate::websocket::EventType;
use crate::*;
use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use futures::sink::SinkExt;
use futures::stream::SplitSink;
use some_transport::{InMemTransportReceiver, Transport};
use std::net::SocketAddr;
use tokio::{task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_connection::{ClientId, Connection, ConnectionState};

impl WebSocketFsm {
	/// Subscribe a connection to specific event types
	pub async fn subscribe_connection(&self, conn_id: &str, event_types: Vec<EventType>) -> ConnectionReceivers {
		let receivers = ConnectionReceivers::new();

		for e in event_types {
			let subject = e.connection_subject(conn_id);
			let receiver = self.transport.subscribe_to_subject(&subject).await;
			receivers.insert(e.clone(), receiver);

			debug!("Connection {} subscribed to {:?} (subject: {})", conn_id, e, subject);
		}

		receivers
	}

	/// Update subscriptions for an existing connection
	pub async fn update_subscriptions(&self, connection_id: &str, add_types: Vec<EventType>, remove_types: Vec<EventType>, receivers: &ConnectionReceivers) {
		// Remove old subsriptions
		for e in remove_types {
			receivers.remove(&e);
		}

		// Add new subscriptios
		for e in add_types {
			let subject = e.connection_subject(connection_id);
			let receiver = self.transport.subscribe_to_subject(&subject).await;
			receivers.insert(e, receiver);
		}
	}
}

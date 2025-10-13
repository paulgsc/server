use super::*;
use crate::WebSocketFsm;
use tokio::time::Duration;

impl WebSocketFsm {
	/// Gracefully disconnect all active connections during shutdown
	pub async fn shutdown(&self) {
		record_system_event!("websocket_shutdown_started");
		info!("Starting WebSocket shutdown - disconnecting all connections");

		let connection_count = self.store.len();

		if connection_count == 0 {
			record_system_event!("websocket_shutdown_completed", connections_disconnected = 0);
			info!("No connections to disconnect");
			self.stop().await; // Stop heartbeat and other tasks
			return;
		}

		// Collect all connection keys
		let connection_keys: Vec<String> = self.store.keys();

		info!("Disconnecting {} active connections", connection_keys.len());

		// Notify all clients of shutdown (best effort)
		let disconnect_event = Event::Error {
			message: "Server is shutting down".to_string(),
		};

		// Broadcast shutdown message to all connections
		let notification_result = tokio::time::timeout(Duration::from_secs(2), self.broadcast_event(&disconnect_event)).await;

		match notification_result {
			Ok(result) => {
				info!("Shutdown notification: {} delivered, {} failed", result.delivered, result.failed);
			}
			Err(_) => {
				warn!("Timeout while notifying connections of shutdown");
			}
		}

		// Give clients a brief moment to process disconnect message
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Remove all connections
		let mut disconnected_count = 0;

		for key in connection_keys {
			match self.remove_connection(&key, "Server shutdown".to_string()).await {
				Ok(()) => {
					disconnected_count += 1;
				}
				Err(e) => {
					warn!("Failed to remove connection {} during shutdown: {}", key, e);
				}
			}
		}

		// Stop heartbeat manager and other background tasks
		self.stop().await;

		record_system_event!("websocket_shutdown_completed", connections_disconnected = disconnected_count);
		info!("WebSocket shutdown completed - disconnected {} connections", disconnected_count);
	}
}

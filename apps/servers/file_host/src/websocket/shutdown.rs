use super::*;
use crate::WebSocketFsm;

impl WebSocketFsm {
	/// Gracefully disconnect all active connections during shutdown
	pub async fn shutdown(&self) {
		record_system_event!("websocket_shutdown_started");
		info!("Starting WebSocket shutdown - disconnecting all connections");

		let connection_count = self.store.len();

		if connection_count == 0 {
			record_system_event!("websocket_shutdown_completed", connections_disconnected = 0);
			info!("No connections to disconnect");
			return;
		}

		// Collect all connection keys
		let connection_keys: Vec<String> = self.store.keys();

		info!("Disconnecting {} active connections", connection_keys.len());

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

		info!("WebSocket shutdown completed - disconnected {} connections", disconnected_count);
	}
}

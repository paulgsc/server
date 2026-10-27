use super::*;
use crate::WebSocketFsm;

impl WebSocketFsm {
	/// Gracefully disconnect all active connections during shutdown
	pub async fn shutdown(&self) {
		info!("Starting WebSocket shutdown - disconnecting all connections");

		let connection_count = self.store.len();

		if connection_count == 0 {
			info!("No connections to disconnect");
			return;
		}

		// Collect all connection keys
		let connection_keys: Vec<String> = self.store.keys();

		info!("Disconnecting {} active connections", connection_keys.len());

		// Remove all connections
		let disconnected_count = connection_keys.len();

		for key in connection_keys {
			self.remove_connection(&key, "Server shutdown".to_string()).await;
		}

		info!("WebSocket shutdown completed - disconnected {} connections", disconnected_count);

		// Give tasks a moment to observe cancellation and clean up
		// This allows the Drop impl in ConnectionCleanup to run
		tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
	}
}

use super::*;
use crate::WebSocketFsm;
use tokio::time::Duration;

impl WebSocketFsm {
	/// Gracefully disconnect all active connections during shutdown
	pub async fn shutdown(&self) {
		record_system_event!("websocket_shutdown_started");
		info!("Starting WebSocket shutdown - disconnecting all connections");

		let connection_count = self.connections.len();
		if connection_count == 0 {
			record_system_event!("websocket_shutdown_completed", connections_disconnected = 0);
			info!("No connections to disconnect");
			return;
		}

		// Collect all connection keys to avoid holding references during iteration
		let connection_keys: Vec<String> = self.connections.iter().map(|entry| entry.key().clone()).collect();

		info!("Disconnecting {} active connections", connection_keys.len());

		// Send disconnect event to all connections in parallel
		// let disconnect_tasks: Vec<_> = connection_keys
		// 	.iter()
		// 	.map(|key| {
		// 		let connections = &self.connections;
		// 		async move {
		// 			if let Some(connection) = connections.get(key) {
		// 				let connection_id = connection.id.clone();
		// 				let disconnect_event = Event::Error {
		// 					message: "Server is shutting down".to_string(),
		// 				};

		// 				// Best effort to notify client
		// 				if let Err(e) = connection.send_event(disconnect_event).await {
		// 					record_ws_error!("shutdown_notify_failed", "shutdown", e);
		// 					warn!("Failed to notify connection {} of shutdown: {}", connection_id, e);
		// 				}
		// 			}
		// 		}
		// 	})
		// 	.collect();

		// Execute notifications concurrently with timeout
		// let notification_timeout = Duration::from_secs(2);
		// match tokio::time::timeout(notification_timeout, futures::future::join_all(disconnect_tasks)).await {
		// 	Ok(_) => {
		// 		info!("Successfully notified all connections of shutdown");
		// 	}
		// 	Err(_) => {
		// 		warn!("Timeout while notifying connections of shutdown");
		// 	}
		// }

		// Give clients a brief moment to process disconnect message
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Remove all connections
		let mut disconnected_count = 0;
		for key in connection_keys {
			if let Some((_, mut connection)) = self.connections.remove(&key) {
				let connection_id = connection.id.clone();
				let client_id = connection.client_id.clone();
				let duration = connection.established_at.elapsed();
				let was_active = connection.is_active();

				// Mark as disconnected
				let _ = connection.disconnect("Server shutdown".to_string());

				// Update metrics
				self.metrics.connection_removed(was_active);

				record_connection_removed!(connection_id, client_id, duration, "Server shutdown");
				disconnected_count += 1;
			}
		}

		// Close the main event sender to signal shutdown to all receivers
		self.sender.close();

		record_system_event!("websocket_shutdown_completed", connections_disconnected = disconnected_count);
		info!("WebSocket shutdown completed - disconnected {} connections", disconnected_count);
	}
}

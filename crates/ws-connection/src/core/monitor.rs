// src/websocket/monitor.rs

use crate::websocket::{orchestrator::ConnectionOrchestrator, store::ConnectionStoreStats, Event};
use async_broadcast::Sender;
use std::{sync::Arc, time::Duration};
use tokio::time::{interval, Interval};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Monitors connections for timeouts and performs cleanup
pub struct TimeoutMonitor {
	orchestrator: Arc<ConnectionOrchestrator>,
	broadcast_sender: Sender<Event>,
	timeout_duration: Duration,
	check_interval: Duration,
	shutdown_token: CancellationToken,
}

#[derive(Debug, Clone)]
pub struct MonitorStats {
	pub cycles_completed: u64,
	pub connections_marked_stale: u64,
	pub connections_cleaned_up: u64,
	pub health_check_failures: u64,
	pub last_cycle_duration: Duration,
}

impl TimeoutMonitor {
	pub fn new(
		orchestrator: Arc<ConnectionOrchestrator>,
		broadcast_sender: Sender<Event>,
		timeout_duration: Duration,
		check_interval: Duration,
		shutdown_token: CancellationToken,
	) -> Self {
		Self {
			orchestrator,
			broadcast_sender,
			timeout_duration,
			check_interval,
			shutdown_token,
		}
	}

	/// Start the timeout monitor with graceful shutdown
	pub fn start(self) -> tokio::task::JoinHandle<MonitorStats> {
		tokio::spawn(async move { self.run().await })
	}

	async fn run(self) -> MonitorStats {
		let mut interval = interval(self.check_interval);
		let mut stats = MonitorStats {
			cycles_completed: 0,
			connections_marked_stale: 0,
			connections_cleaned_up: 0,
			health_check_failures: 0,
			last_cycle_duration: Duration::ZERO,
		};

		info!(
			"Timeout monitor starting with {}s timeout, {}s interval",
			self.timeout_duration.as_secs(),
			self.check_interval.as_secs()
		);

		loop {
			tokio::select! {
			_ = self.shutdown_token.cancelled() => {
										info!("Timeout monitor received shutdown signal");
															break;
																		}
							_ = interval.tick() => {
														let cycle_start = std::time::Instant::now();

																			match self.process_cycle(&mut stats).await {
																											Ok(()) => {
																																				stats.cycles_completed += 1;
																																											stats.last_cycle_duration = cycle_start.elapsed();

																																																		if stats.cycles_completed % 100 == 0 {
																																																												info!("Timeout monitor completed {} cycles", stats.cycles_completed);
																																																																		}
																																																							}
																																	Err(e) => {
																																										stats.health_check_failures += 1;
																																																	error!("Error in timeout monitor cycle: {}", e);
																																																						}
																																					}

																								// Yield to prevent blocking other tasks
																								tokio::task::yield_now().await;
																											}
									}
		}

		info!(
			cycles = stats.cycles_completed,
			marked_stale = stats.connections_marked_stale,
			cleaned_up = stats.connections_cleaned_up,
			failures = stats.health_check_failures,
			"Timeout monitor shutting down gracefully"
		);

		stats
	}

	async fn process_cycle(&self, stats: &mut MonitorStats) -> Result<(), String> {
		// Health check
		self.check_health().await?;

		// Mark stale connections
		let newly_stale = self.mark_stale_connections().await.map_err(|e| format!("Failed to mark stale connections: {}", e))?;

		stats.connections_marked_stale += newly_stale as u64;

		if newly_stale > 0 {
			info!("Marked {} connections as stale", newly_stale);
		}

		// Clean up stale connections
		let cleaned_up = self.cleanup_stale_connections().await.map_err(|e| format!("Failed to cleanup stale connections: {}", e))?;

		stats.connections_cleaned_up += cleaned_up as u64;

		if cleaned_up > 0 {
			info!("Cleaned up {} stale connections", cleaned_up);

			// Broadcast updated client count
			let count = self.orchestrator.get_connection_count().await;
			if let Err(e) = self.broadcast_sender.broadcast(Event::ClientCount { count }).await {
				warn!("Failed to broadcast client count update: {}", e);
			}
		}

		Ok(())
	}

	async fn check_health(&self) -> Result<(), String> {
		let store_stats = self.orchestrator.get_stats().await;

		// Log health snapshot
		info!(
			total = store_stats.total_connections,
			active = store_stats.active_connections,
			stale = store_stats.stale_connections,
			disconnected = store_stats.disconnected_connections,
			unique_clients = store_stats.unique_clients,
			"Connection health snapshot"
		);

		// Check for clients with excessive connections
		let high_connection_clients = self.find_high_connection_clients().await;

		for (client_id, count) in high_connection_clients {
			if count > 10 {
				warn!(
				client_id = %client_id,
									connection_count = count,
														"Client has high connection count"
																	);
				// record_system_event!("client_high_connection_count", client_id = client_id, count = count);
			}
		}

		// Validate store consistency
		self.validate_store_consistency(&store_stats).await?;

		Ok(())
	}

	async fn find_high_connection_clients(&self) -> Vec<(crate::websocket::types::ClientId, usize)> {
		// This would need to be implemented based on your actual store interface
		// For now, we'll return empty vec as placeholder
		Vec::new()
	}

	async fn validate_store_consistency(&self, stats: &ConnectionStoreStats) -> Result<(), String> {
		// Basic consistency checks
		let expected_total = stats.active_connections + stats.stale_connections + stats.disconnected_connections;

		if expected_total != stats.total_connections {
			let error = format!(
				"Connection state count mismatch: expected {}, got {} (active: {}, stale: {}, disconnected: {})",
				expected_total, stats.total_connections, stats.active_connections, stats.stale_connections, stats.disconnected_connections
			);
			error!("{}", error);
			return Err(error);
		}

		Ok(())
	}

	async fn mark_stale_connections(&self) -> Result<usize, crate::websocket::errors::ConnectionError> {
		// Find connections that should be stale
		let stale_keys = self.orchestrator.find_connections(|conn| conn.should_be_stale(self.timeout_duration)).await;

		let mut newly_stale = 0;

		// Process in chunks to avoid blocking
		for chunk in stale_keys.chunks(10) {
			for key in chunk {
				if let Err(e) = self.orchestrator.mark_stale(key, "Connection timeout".to_string()).await {
					match e {
						crate::websocket::errors::ConnectionError::NotFound => {
							// Connection was already removed, ignore
						}
						crate::websocket::errors::ConnectionError::InvalidTransition { .. } => {
							// Connection already stale/disconnected, ignore
						}
						_ => {
							warn!("Failed to mark connection {} as stale: {}", key, e);
						}
					}
				} else {
					newly_stale += 1;
				}
			}

			// Yield after each chunk
			tokio::task::yield_now().await;
		}

		Ok(newly_stale)
	}

	async fn cleanup_stale_connections(&self) -> Result<usize, crate::websocket::errors::ConnectionError> {
		// Find stale connections to clean up
		let stale_keys = self.orchestrator.find_connections(|conn| conn.is_stale()).await;

		let mut cleaned_up = 0;

		// Process in chunks to avoid blocking
		for chunk in stale_keys.chunks(64) {
			for key in chunk {
				if let Err(e) = self.orchestrator.remove_connection(key, "Stale connection cleanup".to_string()).await {
					match e {
						crate::websocket::errors::ConnectionError::NotFound => {
							// Connection was already removed, ignore
						}
						_ => {
							warn!("Failed to remove stale connection {}: {}", key, e);
						}
					}
				} else {
					cleaned_up += 1;
				}
			}

			// Yield after each chunk
			tokio::task::yield_now().await;
		}

		Ok(cleaned_up)
	}
}

/// Builder for TimeoutMonitor with sensible defaults
pub struct TimeoutMonitorBuilder {
	orchestrator: Option<Arc<ConnectionOrchestrator>>,
	broadcast_sender: Option<Sender<Event>>,
	timeout_duration: Duration,
	check_interval: Duration,
	shutdown_token: Option<CancellationToken>,
}

impl TimeoutMonitorBuilder {
	pub fn new() -> Self {
		Self {
			orchestrator: None,
			broadcast_sender: None,
			timeout_duration: Duration::from_secs(300), // 5 minutes default
			check_interval: Duration::from_secs(30),    // 30 seconds default
			shutdown_token: None,
		}
	}

	pub fn orchestrator(mut self, orchestrator: Arc<ConnectionOrchestrator>) -> Self {
		self.orchestrator = Some(orchestrator);
		self
	}

	pub fn broadcast_sender(mut self, sender: Sender<Event>) -> Self {
		self.broadcast_sender = Some(sender);
		self
	}

	pub fn timeout_duration(mut self, duration: Duration) -> Self {
		self.timeout_duration = duration;
		self
	}

	pub fn check_interval(mut self, interval: Duration) -> Self {
		self.check_interval = interval;
		self
	}

	pub fn shutdown_token(mut self, token: CancellationToken) -> Self {
		self.shutdown_token = Some(token);
		self
	}

	pub fn build(self) -> Result<TimeoutMonitor, String> {
		let orchestrator = self.orchestrator.ok_or("Orchestrator is required")?;

		let broadcast_sender = self.broadcast_sender.ok_or("Broadcast sender is required")?;

		let shutdown_token = self.shutdown_token.ok_or("Shutdown token is required")?;

		Ok(TimeoutMonitor::new(
			orchestrator,
			broadcast_sender,
			self.timeout_duration,
			self.check_interval,
			shutdown_token,
		))
	}
}

impl Default for TimeoutMonitorBuilder {
	fn default() -> Self {
		Self::new()
	}
}

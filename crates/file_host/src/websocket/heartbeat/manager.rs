use super::HeartbeatPolicy;
use crate::metrics::ConnectionMetrics;
use crate::transport::TransportLayer;
use crate::websocket::Event;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};
use ws_connection::{ConnectionStore, EventKey};

const BATCH_SIZE: usize = 32; // scalable default

pub struct HeartbeatManager<K: EventKey + Send + Sync + 'static> {
	store: Arc<ConnectionStore<K>>,
	transport: Arc<TransportLayer>,
	metrics: Arc<ConnectionMetrics>,
	policy: HeartbeatPolicy,
	shutdown_tx: watch::Sender<bool>,
}

impl<K: EventKey + Send + Sync + 'static> HeartbeatManager<K> {
	pub fn new(store: Arc<ConnectionStore<K>>, transport: Arc<TransportLayer>, metrics: Arc<ConnectionMetrics>, policy: HeartbeatPolicy) -> Self {
		let (shutdown_tx, _) = watch::channel(false);
		Self {
			store,
			transport,
			metrics,
			policy,
			shutdown_tx,
		}
	}

	/// Spawn the periodic scanner
	pub fn spawn(self: Arc<Self>) -> JoinHandle<()> {
		let policy = self.policy.clone();
		let store = self.store.clone();
		let transport = self.transport.clone();
		let metrics = self.metrics.clone();
		let mut shutdown_rx = self.shutdown_tx.subscribe();

		tokio::spawn(async move {
			let mut interval = tokio::time::interval(policy.scan_interval);
			info!("HeartbeatManager started with policy: {:?}", policy);

			loop {
				tokio::select! {
						_ = shutdown_rx.changed() => {
								if *shutdown_rx.borrow() {
										info!("HeartbeatManager shutting down");
										break;
								}
						}
						_ = interval.tick() => {
								if let Err(e) = Self::run_cycle(&*store, &*transport, &*metrics, &policy).await {
										error!("Heartbeat cycle failed: {}", e);
								}
								tokio::task::yield_now().await;
						}
				}
			}
			info!("HeartbeatManager stopped");
		})
	}

	/// Request graceful shutdown
	pub async fn shutdown(&self) {
		let _ = self.shutdown_tx.send(true);
	}

	/// Record an incoming ping - just send message to actor
	pub async fn record_ping(&self, key: &str) {
		if let Some(handle) = self.store.get(key) {
			if let Err(e) = handle.record_activity().await {
				error!("Failed to record ping for {}: {}", key, e);
			}
		} else {
			debug!("record_ping: connection {} not found", key);
		}
	}

	/// Main heartbeat cycle
	async fn run_cycle(store: &ConnectionStore<K>, transport: &TransportLayer, metrics: &ConnectionMetrics, policy: &HeartbeatPolicy) -> Result<(), String> {
		// Health check
		Self::health_check(store, metrics).await;

		// Mark stale connections
		let newly_stale = Self::mark_stale_connections(store, metrics, policy).await;
		if newly_stale > 0 {
			info!("Marked {} connections as stale", newly_stale);
		}

		// Remove stale connections that exceeded grace period
		let removed = Self::remove_stale_connections(store, transport, policy).await?;
		if removed > 0 {
			info!("Removed {} stale connections", removed);
			// Broadcast updated count
			let count = store.len();
			let _ = transport.broadcast(Event::ClientCount { count }).await;
		}

		Ok(())
	}

	async fn health_check(store: &ConnectionStore<K>, metrics: &ConnectionMetrics) {
		let stats = store.stats().await;
		let snapshot = metrics.get_snapshot();

		debug!(
			"Heartbeat health: total={}, active={}, stale={}, clients={}",
			stats.total_connections, stats.active_connections, stats.stale_connections, stats.unique_clients
		);

		// Check invariants
		let expected_active = snapshot.total_created - snapshot.total_removed;
		if stats.total_connections as u64 != expected_active {
			tracing::warn!("Connection count mismatch: expected={}, actual={}", expected_active, stats.total_connections);
		}
	}

	async fn mark_stale_connections(store: &ConnectionStore<K>, metrics: &ConnectionMetrics, policy: &HeartbeatPolicy) -> usize {
		let keys = store.keys();
		let mut newly_stale = 0;

		for (idx, key) in keys.iter().enumerate() {
			if let Some(handle) = store.get(key) {
				// Ask actor to check if it should be stale
				if let Err(e) = handle.check_stale(policy.stale_after).await {
					debug!("Failed to check stale for {}: {}", key, e);
				} else {
					// Check if it became stale
					if let Ok(state) = handle.get_state().await {
						if state.is_stale && !state.disconnect_reason.is_some() {
							metrics.connection_marked_stale();
							newly_stale += 1;
							record_connection_state_change!(key, "active", "stale");
						}
					}
				}
			}

			// Deterministic batch yield with random jitter
			if idx % BATCH_SIZE == 0 && rng.gen_bool(0.5) {
				tokio::task::yield_now().await;
			}
		}

		newly_stale
	}

	async fn remove_stale_connections(store: &ConnectionStore<K>, transport: &TransportLayer, policy: &HeartbeatPolicy) -> Result<usize, String> {
		let keys = store.keys();
		let mut to_remove = Vec::new();

		// Find stale connections that exceeded grace period
		for key in keys {
			if let Some(handle) = store.get(&key) {
				if let Ok(state) = handle.get_state().await {
					if state.is_stale {
						// Check if stale duration exceeded
						let stale_duration = std::time::Instant::now().duration_since(state.last_activity);

						let total_timeout = policy.stale_after + policy.remove_after_stale;
						if stale_duration > total_timeout {
							to_remove.push(key);
						}
					}
				}
			}
		}

		let mut removed = 0;
		for chunk in to_remove.chunks(64) {
			for key in chunk {
				if let Some(handle) = store.remove(key).await {
					// Send disconnect command to actor
					let _ = handle.disconnect("Stale connection cleanup".to_string()).await;

					// Clean up transport
					transport.remove_channel(key).await;

					removed += 1;
					record_connection_removed!(key, handle.connection.client_id, handle.connection.get_duration(), "stale_cleanup");
				}
			}
			tokio::task::yield_now().await;
		}

		Ok(removed)
	}
}

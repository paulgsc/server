use super::HeartbeatPolicy;
use crate::websocket::*;
use std::sync::Arc;
use tokio::{task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use ws_connection::{ConnectionStore, EventKey};

const BATCH_SIZE: usize = 32; // scalable default

type T = Arc<InMemTransport<Event>>;

pub struct HeartbeatManager<K: EventKey + Send + Sync + 'static> {
	store: Arc<ConnectionStore<K>>,
	transport: T,
	metrics: Arc<ConnectionMetrics>,
	policy: HeartbeatPolicy,
	cancel_token: CancellationToken,
}

impl<K: EventKey + Send + Sync + 'static> HeartbeatManager<K> {
	pub fn new(store: Arc<ConnectionStore<K>>, transport: T, metrics: Arc<ConnectionMetrics>, policy: HeartbeatPolicy, parent_token: &CancellationToken) -> Self {
		Self {
			store,
			transport,
			metrics,
			policy,
			cancel_token: parent_token.child_token(),
		}
	}

	/// Spawn the periodic scanner
	pub fn spawn(self: Arc<Self>) -> JoinHandle<()> {
		let store = self.store.clone();
		let transport = self.transport.clone();
		let metrics = self.metrics.clone();
		let policy = self.policy.clone();
		let token = self.cancel_token.clone();

		tokio::spawn(async move {
			let mut interval = tokio::time::interval(policy.scan_interval);
			info!("HeartbeatManager started with policy: {:?}", policy);

			loop {
				tokio::select! {
					_ = token.cancelled() => {
						info!("HeartbeatManager shutting down via cancellation token");
						break;
					}
					_ = interval.tick() => {
						if let Err(e) = Self::run_cycle(&store, &transport, &metrics, &policy, &token).await {
							error!("Heartbeat cycle failed: {}", e);
						}
					}
				}
			}

			info!("HeartbeatManager stopped");
		})
	}

	/// Request graceful shutdown
	pub async fn shutdown(&self) {
		self.cancel_token.cancel();
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
	async fn run_cycle(store: &ConnectionStore<K>, transport: &T, metrics: &ConnectionMetrics, policy: &HeartbeatPolicy, token: &CancellationToken) -> Result<(), String> {
		Self::health_check(store, metrics).await;
		let removed = Self::check_and_remove_stale(store, transport, metrics, policy, token).await?;
		if removed > 0 {
			info!("Removed {} stale connections", removed);
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

	async fn check_and_remove_stale(
		store: &ConnectionStore<K>,
		transport: &T,
		metrics: &ConnectionMetrics,
		policy: &HeartbeatPolicy,
		token: &CancellationToken,
	) -> Result<usize, String> {
		let keys = store.keys();
		let mut to_remove = Vec::new();

		for (idx, key) in keys.iter().enumerate() {
			if token.is_cancelled() {
				break;
			}

			if let Some(handle) = store.get(key) {
				if let Ok(state) = handle.get_state().await {
					let inactive_duration = Instant::now().duration_since(state.last_activity);

					// Remove if inactive for longer than stale_after period
					if inactive_duration > policy.stale_after && state.disconnect_reason.is_none() {
						to_remove.push(key.clone());
						metrics.connection_marked_stale();
						debug!("Connection {} is stale (inactive for {:?}), will remove", key, inactive_duration);
					}
				}
			}

			if idx % BATCH_SIZE == 0 {
				tokio::task::yield_now().await;
			}
		}

		let mut removed = 0;
		for chunk in to_remove.chunks(64) {
			for key in chunk {
				if token.is_cancelled() {
					break;
				}

				if let Some(handle) = store.remove(key).await {
					let _ = handle.disconnect("Stale connection cleanup".to_string()).await;
					transport.close_channel(key).await.map_err(|e| e.to_string())?;
					removed += 1;
					record_connection_removed!(key, handle.connection.client_id, handle.connection.get_duration(), "stale_cleanup");
				}
			}
			tokio::task::yield_now().await;
		}

		Ok(removed)
	}
}

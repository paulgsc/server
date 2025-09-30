use std::time::{Duration, Instant};
use tokio::{task::JoinHandle, time::interval};
use tokio_util::sync::CancellationToken;

/// Basic connection state
#[derive(Debug, Clone)]
pub enum ConnectionState {
	Active { last_ping: Instant },
	Stale { since: Instant },
	Disconnected { reason: String, at: Instant },
}

/// Minimal config
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
	pub stale_timeout: Duration,
	pub disconnect_timeout: Duration,
	pub sweep_interval: Duration,
}

impl Default for CoordinatorConfig {
	fn default() -> Self {
		Self {
			stale_timeout: Duration::from_secs(30),
			disconnect_timeout: Duration::from_secs(60),
			sweep_interval: Duration::from_secs(10),
		}
	}
}

/// Core broker
pub struct Coordinator {
	store: ConnectionStore,
	shutdown: CancellationToken,
	sweeper: JoinHandle<()>,
	config: CoordinatorConfig,
}

impl Coordinator {
	pub fn new(store: ConnectionStore, config: CoordinatorConfig) -> Self {
		let shutdown = CancellationToken::new();
		let store_clone = store.clone();
		let shutdown_clone = shutdown.clone();

		let sweeper = tokio::spawn(async move {
			let mut ticker = interval(config.sweep_interval);

			loop {
				tokio::select! {
						_ = ticker.tick() => {
								let now = Instant::now();
								for key in store_clone.keys() {
										if let Some(conn) = store_clone.get(&key) {
												match conn.state {
														ConnectionState::Active { last_ping }
														if now.duration_since(last_ping) > config.stale_timeout =>
														{
																store_clone.mark_stale(&key, now);
														}
														ConnectionState::Stale { since }
														if now.duration_since(since) > config.disconnect_timeout =>
														{
																store_clone.disconnect(&key, "timeout");
														}
														_ => {}
												}
										}
								}
						}
						_ = shutdown_clone.cancelled() => break,
				}
			}
		});

		Self { store, shutdown, sweeper, config }
	}

	pub fn store(&self) -> &ConnectionStore {
		&self.store
	}

	pub fn shutdown(&self) {
		self.shutdown.cancel();
	}

	pub async fn join(self) {
		let _ = self.sweeper.await;
	}
}

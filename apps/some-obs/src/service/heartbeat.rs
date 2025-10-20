use crate::ObsNatsService;
use obs_websocket::RetryConfig;
use std::sync::Arc;
use tokio::time::{interval, Duration};

impl ObsNatsService {
	/// Spawn task for periodic health checks
	pub fn spawn_health_checker(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			tracing::info!("💓 Starting health checker");

			let mut check_interval = interval(self.config.health_check_interval);
			check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

			loop {
				tokio::select! {
						_ = self.cancel_token.cancelled() => {
								tracing::info!("🛑 Health checker shutting down");
								break;
						}
						_ = check_interval.tick() => {
								match self.obs_manager.is_healthy().await {
										Ok(true) => {
												tracing::debug!("💚 OBS connection healthy");
										}
										Ok(false) => {
												tracing::warn!("💔 OBS connection unhealthy");
										}
										Err(e) => {
												tracing::error!("❌ Health check failed: {}", e);
										}
								}
						}
				}
			}

			tracing::info!("✅ Health checker stopped");
		})
	}

	/// Calculate retry delay with exponential backoff
	pub fn calculate_retry_delay(&self) -> Duration {
		// Simple exponential backoff - could be enhanced with jitter
		let retry_config = RetryConfig::default();
		let base_delay = retry_config.initial_delay;
		let max_delay = retry_config.max_delay;

		// For now, use max_delay as a reasonable default
		// In production, track retry attempts for true exponential backoff
		std::cmp::min(base_delay * 2, max_delay)
	}
}

use crate::{Config, Result};
use obs_websocket::{ObsConfig, ObsWebSocketManager};
use some_transport::NatsTransport;
use std::sync::Arc;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use ws_events::UnifiedEvent;

pub mod command;
pub mod events;
pub mod heartbeat;

/// Main OBS NATS service that bridges OBS WebSocket and NATS
pub struct ObsNatsService {
	config: Config,
	obs_manager: Arc<ObsWebSocketManager>,
	transport: NatsTransport<UnifiedEvent>,
	cancel_token: CancellationToken,
}

impl ObsNatsService {
	/// Create a new OBS NATS service
	///
	/// # Errors
	///
	/// Fails when NATS at `config.nats_url` is unreachable.
	pub async fn new(config: Config) -> Result<Self> {
		tracing::info!("🔌 Initializing OBS NATS Service");

		// Create OBS manager (OBS_HOST / OBS_PORT / OBS_PASSWORD)
		let obs_manager = Arc::new(ObsWebSocketManager::new(ObsConfig::from_env()));

		// Create NATS transports using pooled connections
		tracing::info!("📡 Connecting to NATS at {}", config.nats_url);

		let transport = NatsTransport::connect_pooled(&config.nats_url).await?;

		tracing::info!("✅ NATS transports initialized");

		Ok(Self {
			config,
			obs_manager,
			transport,
			cancel_token: CancellationToken::new(),
		})
	}

	/// Run the service until shutdown
	///
	/// # Errors
	///
	/// None at present: task failures are logged and retried, not returned.
	pub async fn run(self) -> Result<()> {
		let service = Arc::new(self);

		// Setup graceful shutdown handler
		let shutdown_token = service.cancel_token.clone();
		tokio::spawn(async move {
			match tokio::signal::ctrl_c().await {
				Ok(()) => {
					tracing::info!("🛑 Shutdown signal received");
					shutdown_token.cancel();
				}
				Err(e) => {
					tracing::error!("❌ Failed to listen for shutdown signal: {}", e);
				}
			}
		});

		// Spawn service tasks
		let command_handler = service.clone().spawn_command_handler();
		let event_bridge = service.clone().spawn_event_bridge();
		let health_checker = service.clone().spawn_health_checker();

		// Wait for shutdown signal
		service.cancel_token.cancelled().await;
		tracing::info!("🔄 Initiating graceful shutdown...");

		// Give tasks time to complete gracefully
		let shutdown_timeout = service.config.shutdown_timeout;
		let _ = timeout(shutdown_timeout, async {
			let _ = tokio::join!(command_handler, event_bridge, health_checker);
		})
		.await;

		// Disconnect from OBS
		service.obs_manager.disconnect().await;

		tracing::info!("✅ Graceful shutdown complete");
		Ok(())
	}
}

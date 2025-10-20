use crate::{Config, ObsCommandMessage, ObsEventMessage, Result};
use obs_websocket::{ObsConfig, ObsWebSocketManager, RetryConfig};
use some_transport::NatsTransport;
use std::sync::Arc;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

pub mod command;
pub mod events;
pub mod heartbeat;

/// Main OBS NATS service that bridges OBS WebSocket and NATS
pub struct ObsNatsService {
	config: Config,
	obs_manager: Arc<ObsWebSocketManager>,
	command_transport: NatsTransport<ObsCommandMessage>,
	event_transport: NatsTransport<ObsEventMessage>,
	cancel_token: CancellationToken,
}

impl ObsNatsService {
	/// Create a new OBS NATS service
	pub async fn new(config: Config) -> Result<Self> {
		tracing::info!("🔌 Initializing OBS NATS Service");

		// Create OBS manager
		let obs_manager = Arc::new(ObsWebSocketManager::new(ObsConfig::default(), RetryConfig::default()));

		// Create NATS transports using pooled connections
		tracing::info!("📡 Connecting to NATS at {}", config.nats_url);

		let command_transport = NatsTransport::connect_pooled(&config.nats_url).await?;
		let event_transport = NatsTransport::connect_pooled(&config.nats_url).await?;

		tracing::info!("✅ NATS transports initialized");

		Ok(Self {
			config,
			obs_manager,
			command_transport,
			event_transport,
			cancel_token: CancellationToken::new(),
		})
	}

	/// Run the service until shutdown
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
		if let Err(e) = service.obs_manager.disconnect().await {
			tracing::warn!("⚠️ Error disconnecting from OBS: {}", e);
		}

		tracing::info!("✅ Graceful shutdown complete");
		Ok(())
	}
}

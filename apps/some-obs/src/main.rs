#![allow(clippy::multiple_crate_versions)]
use some_obs::{Config, ObsNatsService};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Initialize tracing
	tracing_subscriber::registry()
		.with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "obs_nats_service=info,obs_websocket=info".into()))
		.with(tracing_subscriber::fmt::layer())
		.init();

	tracing::info!("🚀 Starting OBS NATS Service");

	// Load configuration from environment or use defaults
	let config = Config::from_env()?;

	tracing::info!("📋 Configuration loaded - OBS , NATS: {}", config.nats_url);

	// Create and run the service
	let service = ObsNatsService::new(config).await?;

	// Run until shutdown signal
	service.run().await?;

	tracing::info!("👋 OBS NATS Service shutdown complete");
	Ok(())
}

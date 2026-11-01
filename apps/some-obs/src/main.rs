use some_obs::{Config, ObsNatsService};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Before anything else: a value that does not parse stops startup here.
	let config: Config = app_config::schema::parse(&app_config::schema::NO_RULES);

	tracing_subscriber::registry()
		.with(tracing_subscriber::EnvFilter::try_new(&config.log.rust_log)?)
		.with(tracing_subscriber::fmt::layer())
		.init();

	tracing::info!("🚀 Starting OBS NATS Service");

	tracing::info!("📋 Configuration loaded - OBS , NATS: {}", config.nats.nats_url);

	// Create and run the service
	let service = ObsNatsService::new(&config).await?;

	// Run until shutdown signal
	service.run().await?;

	tracing::info!("👋 OBS NATS Service shutdown complete");
	Ok(())
}

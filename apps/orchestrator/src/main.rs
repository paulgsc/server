use anyhow::Result;
use clap::Parser;
use orchestrator::{config::Config, OrchestratorService};
use some_transport::NatsTransport;
use tracing_subscriber::EnvFilter;
use ws_events::events::UnifiedEvent;

#[tokio::main]
async fn main() -> Result<()> {
	let config = Config::parse();

	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::try_new(&config.rust_log)?)
		.with_target(true)
		.with_line_number(true)
		.init();

	tracing::info!("🎬 Starting NATS Orchestrator Service");

	let metrics_handle = some_metrics::install()?;
	let metrics_addr = config.metrics_addr;
	tokio::spawn(async move {
		if let Err(error) = some_metrics::serve(metrics_addr, metrics_handle).await {
			tracing::error!(%error, "metrics listener exited");
		}
	});

	tracing::info!("📡 Connecting to NATS at {}", config.nats_url);

	// Create NATS transport using the pooled connection
	let transport: NatsTransport<UnifiedEvent> = NatsTransport::connect_pooled(&config.nats_url).await?;
	tracing::info!("✅ Connected to NATS");
	tracing::info!("   - Commands: listening on {}", ws_events::events::EventType::OrchestratorCommandData.subject());
	tracing::info!("   - State: publishing on {}", ws_events::events::EventType::OrchestratorState.subject());

	let service = OrchestratorService::new(transport);
	tracing::info!("🎯 Service initialized");

	// Setup signal handling for graceful shutdown
	let service_shutdown = service.clone();
	tokio::spawn(async move {
		match tokio::signal::ctrl_c().await {
			Ok(()) => {
				tracing::info!("🛑 Received shutdown signal (Ctrl+C)");
				service_shutdown.shutdown();
			}
			Err(e) => {
				tracing::error!("Failed to listen for shutdown signal: {}", e);
			}
		}
	});

	tracing::info!("🚀 Orchestrator service running");
	tracing::info!("   Waiting for OrchestratorCommandData events on NATS...");

	// Run the service
	if let Err(e) = service.run().await {
		tracing::error!("❌ Service error: {}", e);
		return Err(e);
	}

	tracing::info!("👋 Orchestrator service stopped gracefully");
	Ok(())
}

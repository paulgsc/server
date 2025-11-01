use orchestrator::OrchestratorService;
use some_transport::NatsTransport;
use tracing::Level;
use ws_events::events::UnifiedEvent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Initialize tracing
	tracing_subscriber::fmt()
		.with_max_level(Level::INFO)
		.with_target(true)
		.with_thread_ids(true)
		.with_line_number(true)
		.init();

	tracing::info!("🎬 Starting NATS Orchestrator Service");

	// Get NATS URL from environment or use default
	let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
	tracing::info!("📡 Connecting to NATS at {}", nats_url);

	// Create NATS transports using the pooled connection
	// This ensures all transports share the same underlying NATS connection
	let transport: NatsTransport<UnifiedEvent> = NatsTransport::connect_pooled(&nats_url).await?;

	tracing::info!("✅ Connected to NATS");
	tracing::info!("   - Commands: broadcasting on NATS");
	tracing::info!("   - Subscriptions: broadcasting on NATS");
	tracing::info!("   - State updates: will publish per-stream");

	let service = OrchestratorService::new(transport);

	tracing::info!("🎯 Service initialized with actor-based subscriber management");

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
	tracing::info!("   Waiting for commands...");

	// Run the service
	if let Err(e) = service.run().await {
		tracing::error!("❌ Service error: {}", e);
		return Err(e);
	}

	tracing::info!("👋 Orchestrator service stopped gracefully");
	Ok(())
}

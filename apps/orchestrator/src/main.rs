mod managed_orchestrator;
mod service;
mod types;

use service::OrchestratorService;
use some_transport::{NatsReceiver, NatsTransport};
use std::sync::Arc;
use tracing::Level;
use types::{OrchestratorCommand, StateUpdate, SubscriptionCommand};

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
	let state_transport: NatsTransport<StateUpdate> = NatsTransport::connect_pooled(&nats_url).await?;

	let command_transport: NatsTransport<OrchestratorCommand> = NatsTransport::connect_pooled(&nats_url).await?;

	let subscription_transport: NatsTransport<SubscriptionCommand> = NatsTransport::connect_pooled(&nats_url).await?;

	tracing::info!("✅ Connected to NATS");

	// Subscribe to command and subscription channels
	tracing::info!("📥 Subscribing to command and subscription channels");
	let command_rx = command_transport.subscribe().await;
	let subscription_rx = subscription_transport.subscribe().await;

	tracing::info!("✅ Subscribed to NATS channels");
	tracing::info!("   - Commands: broadcasting on NATS");
	tracing::info!("   - Subscriptions: broadcasting on NATS");
	tracing::info!("   - State updates: will publish per-stream");

	// Create the orchestrator service
	// Note: We use state_transport for broadcasting state updates
	let service = Arc::new(OrchestratorService::<NatsTransport<StateUpdate>, NatsReceiver<OrchestratorCommand>>::new(state_transport));

	tracing::info!("🎯 Service initialized with actor-based subscriber management");

	// Setup signal handling for graceful shutdown
	let service_shutdown = Arc::clone(&service);
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
	if let Err(e) = service.run(command_rx, subscription_rx).await {
		tracing::error!("❌ Service error: {}", e);
		return Err(e);
	}

	tracing::info!("👋 Orchestrator service stopped gracefully");
	Ok(())
}

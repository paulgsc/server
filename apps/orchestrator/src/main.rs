use anyhow::Result;
use clap::Parser;
use orchestrator::OrchestratorService;
use some_transport::NatsTransport;
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;
use ws_events::events::UnifiedEvent;

/// Everything `orchestrator` reads from its environment (#404).
///
/// Each field is also a flag. A value that does not parse stops startup with
/// an error naming the variable; it never falls back to the default.
#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
struct Config {
	/// Where the Prometheus `/metrics` listener binds.
	///
	/// Orchestrator has no other HTTP surface (#143 (C1)) — this listener
	/// exists purely to be scraped. All interfaces, so it's reachable from
	/// `monitoring-network`, but `infra/compose/orchestrator.yml` deliberately
	/// publishes no port to the host, and this must not change that.
	#[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9464")]
	metrics_addr: SocketAddr,

	/// NATS server to connect to.
	///
	/// The default only works beside a NATS that publishes 4222 on the same
	/// machine (a `cargo run` next to the compose stack); the container always
	/// sets it (`infra/compose/orchestrator.yml`).
	#[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
	nats_url: String,

	/// `tracing` filter directives, e.g. `info,async_nats=warn`.
	///
	/// Until #404 this was ignored: the subscriber was fixed at INFO whatever
	/// the compose file set.
	#[arg(long, env = "RUST_LOG", default_value = "info", value_parser = parse_filter)]
	rust_log: String,
}

/// Rejects a filter `EnvFilter` can't parse, so a bad `RUST_LOG` is a clap
/// error naming the variable rather than one raised after parsing.
fn parse_filter(directives: &str) -> Result<String, tracing_subscriber::filter::ParseError> {
	EnvFilter::try_new(directives).map(|_| directives.to_owned())
}

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

#[cfg(test)]
mod tests {
	use super::Config;
	use clap::{error::ErrorKind, CommandFactory, Parser};

	#[test]
	fn config_definition_is_valid() {
		Config::command().debug_assert();
	}

	/// The old `METRICS_ADDR` read swallowed a parse failure and bound the
	/// default instead. Clap applies the same parser to the flag and the
	/// variable, so the flag stands in for the variable here without touching
	/// the process environment.
	#[test]
	fn an_unparseable_metrics_addr_is_a_boot_error() {
		let err = Config::try_parse_from(["orchestrator", "--metrics-addr", "not-an-address"]).unwrap_err();
		assert_eq!(err.kind(), ErrorKind::ValueValidation);
	}

	/// `RUST_LOG` was ignored before #404; now a value it can't use is
	/// refused rather than silently dropped.
	#[test]
	fn an_unparseable_rust_log_is_a_boot_error() {
		let err = Config::try_parse_from(["orchestrator", "--rust-log", "info,=[bad"]).unwrap_err();
		assert_eq!(err.kind(), ErrorKind::ValueValidation);
	}
}

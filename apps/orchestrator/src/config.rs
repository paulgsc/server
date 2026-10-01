//! Everything `orchestrator` reads from its environment (#404).

use clap::Parser;
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

/// The orchestrator's whole configuration.
///
/// Each field is also a flag. A value that does not parse stops startup with
/// an error naming the variable; it never falls back to the default.
#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Config {
	/// Where the Prometheus `/metrics` listener binds.
	///
	/// Orchestrator has no other HTTP surface (#143 (C1)) — this listener
	/// exists purely to be scraped. All interfaces, so it's reachable from
	/// `monitoring-network`, but `infra/compose/orchestrator.yml` deliberately
	/// publishes no port to the host, and this must not change that.
	#[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9464")]
	pub metrics_addr: SocketAddr,

	/// NATS server to connect to.
	///
	/// The default only works beside a NATS that publishes 4222 on the same
	/// machine (a `cargo run` next to the compose stack); the container always
	/// sets it (`infra/compose/orchestrator.yml`).
	#[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
	pub nats_url: String,

	/// `tracing` filter directives, e.g. `info,async_nats=warn`.
	///
	/// Until #404 this was ignored: the subscriber was fixed at INFO whatever
	/// the compose file set.
	#[arg(long, env = "RUST_LOG", default_value = "info", value_parser = parse_filter)]
	pub rust_log: String,
}

/// Rejects a filter `EnvFilter` can't parse, so a bad `RUST_LOG` is a clap
/// error naming the variable rather than one raised after parsing.
fn parse_filter(directives: &str) -> Result<String, tracing_subscriber::filter::ParseError> {
	EnvFilter::try_new(directives).map(|_| directives.to_owned())
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

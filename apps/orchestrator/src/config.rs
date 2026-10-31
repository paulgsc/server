//! Everything `orchestrator` reads from its environment (#404).

use app_config::{LogArgs, NatsArgs};
use clap::Parser;
use std::net::SocketAddr;

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

	/// `NATS_URL`. The container always sets it (`infra/compose/orchestrator.yml`).
	#[command(flatten)]
	pub nats: NatsArgs,

	#[command(flatten)]
	pub log: LogArgs,
}

#[cfg(test)]
mod tests {
	use super::Config;
	use clap::CommandFactory;

	#[test]
	fn config_definition_is_valid() {
		Config::command().debug_assert();
	}

	/// ENV1 (README.md, "Configuration"); see `app_config::env1`.
	#[test]
	fn every_variable_that_does_not_parse_is_refused_by_name() {
		app_config::env1::assert_every_variable_is_refused_by_name::<Config>("orchestrator", &[("METRICS_ADDR", "not-an-address"), ("RUST_LOG", "info,=[bad")], &["NATS_URL"]);
	}

	/// CFG7 (#503): `infra/env/orchestrator.json` is this `Config`.
	#[test]
	fn the_checked_in_schema_is_this_config() {
		let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infra/env/orchestrator.json");
		app_config::schema::assert_checked_in("orchestrator", &Config::command(), &app_config::schema::NO_RULES, &path);
		app_config::schema::assert_groups_hold_only_their_own(&Config::command());
	}
}

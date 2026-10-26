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
	/// machine (a `cargo run` next to the compose stack; HOST-SPECIFIC, see
	/// `some_transport::DEV_HOST_NATS_URL`); the container always sets it
	/// (`infra/compose/orchestrator.yml`).
	#[arg(long, env = "NATS_URL", default_value = some_transport::DEV_HOST_NATS_URL)]
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
	use std::any::TypeId;

	#[test]
	fn config_definition_is_valid() {
		Config::command().debug_assert();
	}

	/// ENV1 (README.md, "Configuration"), for every variable this `Config`
	/// declares: one whose value can fail to parse is refused, by an error
	/// naming it, and one that can't is listed as free text. A variable added
	/// to neither list fails here.
	///
	/// Clap applies the same parser to the flag and the variable, so the flag
	/// stands in for the variable without touching the process environment.
	/// Its error names the argument by its value name, never by the variable
	/// a value came from, so each variable is its own value name.
	#[test]
	fn every_variable_that_does_not_parse_is_refused_by_name() {
		const REFUSED: [(&str, &str); 2] = [("METRICS_ADDR", "not-an-address"), ("RUST_LOG", "info,=[bad")];
		const FREE_TEXT: [&str; 1] = ["NATS_URL"];
		let mut seen = Vec::new();
		for arg in Config::command().get_arguments() {
			let Some(env) = arg.get_env().and_then(std::ffi::OsStr::to_str) else { continue };
			seen.push(env.to_owned());
			let value_names: Vec<&str> = arg.get_value_names().unwrap_or_default().iter().map(clap::builder::Str::as_str).collect();
			assert_eq!(value_names, [env], "{env} is not its own value name, so its errors do not name it");
			let refused: Vec<&str> = REFUSED.iter().filter(|(name, _)| *name == env).map(|(_, bad)| *bad).collect();
			for bad in &refused {
				let flag = String::from("--") + arg.get_long().unwrap() + "=" + bad;
				let err = Config::try_parse_from(["orchestrator", &flag]).unwrap_err();
				assert!(matches!(err.kind(), ErrorKind::ValueValidation | ErrorKind::InvalidValue), "{env}={bad}: {err}");
				assert!(err.to_string().contains(env), "{env}={bad}: {err}");
			}
			if refused.is_empty() {
				assert!(FREE_TEXT.contains(&env), "{env} is in neither REFUSED nor FREE_TEXT");
				assert!(
					arg.get_value_parser().type_id() == TypeId::of::<String>(),
					"{env} is free text, so its parser must be a plain String or PathBuf"
				);
			}
		}
		let mut declared: Vec<&str> = REFUSED.iter().map(|(name, _)| *name).chain(FREE_TEXT).collect();
		declared.sort_unstable();
		declared.dedup();
		seen.sort_unstable();
		assert_eq!(seen, declared, "a listed variable is no longer declared");
	}
}

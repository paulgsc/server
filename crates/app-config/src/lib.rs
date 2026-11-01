//! The configuration groups more than one binary reads, each declared once (#405).
//!
//! Every binary reads its environment through one clap `Config` (#484,
//! README.md, "Configuration"). A variable that means the same thing to more
//! than one of them is declared here, as a `clap::Args` group the binary
//! pulls in with `#[command(flatten)]`, so its name, default and help cannot
//! drift apart between binaries. This crate reads no environment itself.
//!
//! A binary that needs a different default for one variable in a group
//! changes it with `#[command(mut_arg(...))]` on its `Config`, which keeps the
//! name and help here and the exception where it applies.
//!
//! Each group sets its own `--help` heading, and clap's derive never restores
//! the previous one after a `#[command(flatten)]`: a field declared after a
//! group needs a `help_heading` of its own, which
//! [`schema::assert_groups_hold_only_their_own`] holds.

use clap::Args;
use serde::{Deserialize, Serialize};

pub mod env1;
pub mod schema;

pub use some_transport::DEV_HOST_NATS_URL;

/// Where a binary reaches NATS.
#[derive(Args, Clone, Debug, Serialize, Deserialize)]
#[command(next_help_heading = "NATS")]
pub struct NatsArgs {
	/// The NATS server to connect to.
	///
	/// The default only works beside a NATS that publishes 4222 on the same
	/// machine (a `cargo run` next to the compose stack). HOST-SPECIFIC: see
	/// `some_transport::DEV_HOST_NATS_URL`. Every container sets it.
	#[arg(long, env = "NATS_URL", default_value = DEV_HOST_NATS_URL)]
	pub nats_url: String,
}

/// What a binary logs.
#[derive(Args, Clone, Debug, Serialize, Deserialize)]
#[command(next_help_heading = "Logging")]
pub struct LogArgs {
	/// `tracing` filter directives, e.g. `info,async_nats=warn`.
	///
	/// A filter that does not parse stops the binary at start.
	#[arg(long, env = "RUST_LOG", default_value = "info", value_parser = parse_filter)]
	pub rust_log: String,
}

/// Where a binary's traces go.
///
/// The `service.name` traces carry is the binary's own name, and every trace
/// is kept: both are constants of the binary, not variables (#617).
#[derive(Args, Clone, Debug, Serialize, Deserialize)]
#[command(next_help_heading = "Tracing")]
pub struct OtelArgs {
	/// The OTLP (gRPC) collector traces are exported to, e.g.
	/// `http://otel-collector:4317`. Unset, they are not exported at all:
	/// nothing in the compose stack runs a collector, and metrics go through
	/// Prometheus (#139, #407).
	#[arg(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT")]
	pub otel_exporter_otlp_endpoint: Option<String>,

	/// The `service.environment` traces carry.
	#[arg(long, env = "ENVIRONMENT", default_value = "development")]
	pub environment: String,
}

/// Rejects a filter `EnvFilter` can't parse, so a bad `RUST_LOG` is a clap
/// error naming the variable rather than one raised after parsing.
///
/// # Errors
///
/// When `directives` is not a filter `EnvFilter` accepts.
pub fn parse_filter(directives: &str) -> Result<String, tracing_subscriber::filter::ParseError> {
	tracing_subscriber::EnvFilter::try_new(directives).map(|_| directives.to_owned())
}

#[cfg(test)]
mod tests {
	use super::{LogArgs, NatsArgs, OtelArgs, DEV_HOST_NATS_URL};
	use clap::{CommandFactory, Parser};

	#[derive(Parser)]
	#[command(mut_arg("rust_log", |arg| arg.default_value("probe=debug")))]
	struct Probe {
		#[command(flatten)]
		nats: NatsArgs,
		#[command(flatten)]
		log: LogArgs,
		#[command(flatten)]
		otel: OtelArgs,
	}

	#[test]
	fn the_groups_flatten_into_a_valid_config() {
		Probe::command().debug_assert();
	}

	#[test]
	fn a_binary_keeps_the_shared_defaults_and_sets_its_own() {
		let probe = Probe::try_parse_from(["probe"]).unwrap();
		assert_eq!(probe.log.rust_log, "probe=debug");
		assert_eq!(probe.nats.nats_url, DEV_HOST_NATS_URL);
	}

	#[derive(Parser)]
	struct Leaky {
		#[command(flatten)]
		nats: NatsArgs,
		#[arg(long, env = "AFTER_THE_GROUP")]
		after_the_group: Option<String>,
	}

	#[test]
	#[should_panic(expected = "AFTER_THE_GROUP is listed under NATS")]
	fn a_field_after_a_group_without_its_own_heading_is_caught() {
		crate::schema::assert_groups_hold_only_their_own(&Leaky::command());
	}

	#[test]
	fn the_groups_hold_only_their_own_in_a_config_that_names_its_headings() {
		crate::schema::assert_groups_hold_only_their_own(&Probe::command());
	}

	#[derive(Parser)]
	struct Gated {
		#[arg(long = "probe-enabled", env = "PROBE_ENABLED", default_value_t = false, action = clap::ArgAction::Set)]
		enabled: bool,
		#[arg(long = "probe-url", env = "PROBE_URL")]
		url: Option<String>,
		#[arg(long = "probe-token", env = "PROBE_TOKEN")]
		token: Option<String>,
		#[arg(long = "probe-seconds", env = "PROBE_SECONDS", default_value = "60")]
		seconds: u64,
		#[arg(long = "probe-days", env = "PROBE_DAYS", default_value = "90")]
		days: u64,
	}

	const GATED: crate::schema::Rules = crate::schema::Rules {
		required_when: &[crate::schema::RequiredWhen {
			variable: "PROBE_URL",
			when: "PROBE_ENABLED",
			is: "true",
		}],
		bounds: &[
			crate::schema::Bound {
				variable: "PROBE_SECONDS",
				min: Some(60.0),
				max: None,
				when: &[crate::schema::When {
					variable: "PROBE_ENABLED",
					is: "true",
				}],
			},
			crate::schema::Bound {
				variable: "PROBE_DAYS",
				min: Some(30.0),
				max: Some(100.0),
				when: &[
					crate::schema::When {
						variable: "PROBE_ENABLED",
						is: "true",
					},
					crate::schema::When {
						variable: "PROBE_TOKEN",
						is: crate::schema::SET,
					},
				],
			},
		],
	};

	fn unmet(args: &[&str]) -> Vec<String> {
		let cmd = Gated::command();
		let matches = cmd.clone().try_get_matches_from(std::iter::once("probe").chain(args.iter().copied())).unwrap();
		crate::schema::unmet(&cmd, &matches, &GATED)
	}

	/// #580: every rule that does not hold is named, and one whose condition
	/// is off asks for nothing, defaults counted as values.
	#[test]
	fn rules_name_every_gap_while_their_condition_holds() {
		assert!(unmet(&["--probe-seconds", "1", "--probe-days", "1"]).is_empty(), "off, nothing is required or bounded");
		assert_eq!(
			unmet(&["--probe-enabled", "true", "--probe-seconds", "59", "--probe-token", "t", "--probe-days", "101"]),
			[
				"PROBE_URL is required while PROBE_ENABLED is true",
				"PROBE_SECONDS must be at least 60",
				"PROBE_DAYS must be from 30 to 100"
			]
		);
		assert_eq!(
			unmet(&["--probe-enabled", "true", "--probe-url", " "]),
			["PROBE_URL is required while PROBE_ENABLED is true"],
			"blank is unset"
		);
		assert!(
			unmet(&["--probe-enabled", "true", "--probe-url", "u", "--probe-days", "1"]).is_empty(),
			"every condition must hold"
		);
	}

	/// ENV1 for the groups themselves; each binary's own test walks them
	/// again as part of its `Config`.
	#[test]
	fn every_variable_that_does_not_parse_is_refused_by_name() {
		crate::env1::assert_every_variable_is_refused_by_name::<Probe>("probe", &[("RUST_LOG", "info,=[bad")], &["NATS_URL", "OTEL_EXPORTER_OTLP_ENDPOINT", "ENVIRONMENT"]);
	}
}

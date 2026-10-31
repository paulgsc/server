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

/// Where a binary's traces go and how many are kept.
///
/// `OTEL_SERVICE_NAME` has no default here: each binary names itself with
/// [`default_service_name`], so no two binaries' traces share a name by
/// accident.
#[derive(Args, Clone, Debug, Serialize, Deserialize)]
#[command(next_help_heading = "Tracing")]
pub struct OtelArgs {
	/// The `service.name` traces are exported under. Leave it unset: it
	/// defaults to the binary's own name, and a value in a `.env` that more
	/// than one binary loads renames all of their traces.
	#[arg(long, env = "OTEL_SERVICE_NAME")]
	pub otel_service_name: String,

	/// The OTLP (gRPC) collector traces are exported to, e.g.
	/// `http://otel-collector:4317`. Unset, they are not exported at all:
	/// nothing in the compose stack runs a collector, and metrics go through
	/// Prometheus (#139, #407).
	#[arg(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT")]
	pub otel_exporter_otlp_endpoint: Option<String>,

	/// Which traces are kept: all (`always_on`), none (`always_off`), or the
	/// share `OTEL_TRACES_SAMPLER_ARG` names (`traceidratio`).
	#[arg(long, env = "OTEL_TRACES_SAMPLER", value_enum, default_value = "traceidratio")]
	pub otel_traces_sampler: TraceSampler,

	/// With `traceidratio`, the share of traces kept, from 0 to 1.
	#[arg(long, env = "OTEL_TRACES_SAMPLER_ARG", default_value = "1", value_parser = parse_ratio)]
	pub otel_traces_sampler_arg: f64,

	/// The `service.environment` traces carry.
	#[arg(long, env = "ENVIRONMENT", default_value = "development")]
	pub environment: String,
}

/// `OTEL_TRACES_SAMPLER`'s values that are implemented, spelled as the
/// OpenTelemetry spec spells them.
#[derive(clap::ValueEnum, Clone, Copy, Debug, Serialize, Deserialize)]
#[value(rename_all = "snake_case")]
pub enum TraceSampler {
	AlwaysOn,
	AlwaysOff,
	Traceidratio,
}

/// Gives [`OtelArgs`]' `OTEL_SERVICE_NAME` the binary's own name as its
/// default, for `#[command(mut_arg("otel_service_name", default_service_name("file_host")))]`.
pub fn default_service_name(name: &'static str) -> impl FnOnce(clap::Arg) -> clap::Arg {
	move |arg| arg.default_value(name).required(false)
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

/// `OTEL_TRACES_SAMPLER_ARG`'s least value, for a binary's `schema::Bound`.
pub const SAMPLER_ARG_MIN: f64 = 0.0;
/// `OTEL_TRACES_SAMPLER_ARG`'s greatest value.
pub const SAMPLER_ARG_MAX: f64 = 1.0;

fn parse_ratio(ratio: &str) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
	let ratio: f64 = ratio.parse()?;
	if !(SAMPLER_ARG_MIN..=SAMPLER_ARG_MAX).contains(&ratio) {
		return Err("must be from 0 to 1".into());
	}
	Ok(ratio)
}

#[cfg(test)]
mod tests {
	use super::{default_service_name, LogArgs, NatsArgs, OtelArgs, DEV_HOST_NATS_URL};
	use clap::{CommandFactory, Parser};

	#[derive(Parser)]
	#[command(mut_arg("otel_service_name", default_service_name("probe")), mut_arg("rust_log", |arg| arg.default_value("probe=debug")))]
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
	fn a_binary_names_itself_and_keeps_the_shared_defaults() {
		let probe = Probe::try_parse_from(["probe"]).unwrap();
		assert_eq!(probe.otel.otel_service_name, "probe");
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
		ours: &["PROBE_ENABLED", "PROBE_URL"],
		neutral: &["PROBE_SECONDS", "PROBE_DAYS", "PROBE_TOKEN"],
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

	#[derive(Parser)]
	struct Classified {
		#[arg(long, env = "PROBE_SWITCH")]
		switch: Option<String>,
		#[arg(long, env = "PROBE_KEY", hide_env_values = true)]
		key: Option<String>,
	}

	fn classes(ours: &'static [&'static str], neutral: &'static [&'static str]) -> Vec<(String, crate::schema::Class)> {
		let rules = crate::schema::Rules {
			required_when: &[],
			bounds: &[],
			ours,
			neutral,
		};
		crate::schema::schema("probe", &Classified::command(), &rules)
			.variables
			.into_iter()
			.map(|variable| (variable.name, variable.class))
			.collect()
	}

	/// CFG9 (#604): each variable is ours, neutral, or a secret by its
	/// `hide_env_values`, and the schema says which.
	#[test]
	fn every_variable_is_ours_neutral_or_secret() {
		use crate::schema::Class;
		assert_eq!(
			classes(&["PROBE_SWITCH"], &[]),
			[("PROBE_SWITCH".to_owned(), Class::Ours), ("PROBE_KEY".to_owned(), Class::Secret)]
		);
		assert_eq!(classes(&[], &["PROBE_SWITCH"])[0].1, Class::Neutral);
	}

	#[test]
	#[should_panic(expected = "probe does not classify PROBE_SWITCH")]
	fn an_unclassified_variable_is_refused() {
		let _ = classes(&[], &[]);
	}

	#[test]
	#[should_panic(expected = "probe classifies PROBE_SWITCH more than once")]
	fn a_variable_classified_twice_is_refused() {
		let _ = classes(&["PROBE_SWITCH"], &["PROBE_SWITCH"]);
	}

	#[test]
	#[should_panic(expected = "PROBE_KEY is a secret")]
	fn a_secret_classified_as_ours_is_refused() {
		let _ = classes(&["PROBE_SWITCH", "PROBE_KEY"], &[]);
	}

	#[test]
	#[should_panic(expected = "probe classifies PROBE_GONE, which it does not declare")]
	fn a_classification_naming_nothing_is_refused() {
		let _ = classes(&["PROBE_SWITCH", "PROBE_GONE"], &[]);
	}

	/// ENV1 for the groups themselves; each binary's own test walks them
	/// again as part of its `Config`.
	#[test]
	fn every_variable_that_does_not_parse_is_refused_by_name() {
		crate::env1::assert_every_variable_is_refused_by_name::<Probe>(
			"probe",
			&[
				("RUST_LOG", "info,=[bad"),
				("OTEL_TRACES_SAMPLER", "parentbased_always_on"),
				("OTEL_TRACES_SAMPLER_ARG", "half"),
				("OTEL_TRACES_SAMPLER_ARG", "2"),
			],
			&["NATS_URL", "OTEL_SERVICE_NAME", "OTEL_EXPORTER_OTLP_ENDPOINT", "ENVIRONMENT"],
		);
	}
}

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

	/// The OTLP (gRPC) collector traces are exported to.
	#[arg(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT", default_value = "http://localhost:4317")]
	pub otel_exporter_otlp_endpoint: String,

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

fn parse_ratio(ratio: &str) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
	let ratio: f64 = ratio.parse()?;
	if !(0.0..=1.0).contains(&ratio) {
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

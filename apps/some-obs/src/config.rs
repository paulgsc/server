//! Everything `some-obs` reads from its environment (#484).

use app_config::{LogArgs, NatsArgs};
use clap::Parser;
use obs_websocket::ObsConfig;
use std::time::Duration;

/// What some-obs holds its `Config` to beyond its parsers, for the schema and
/// `make preflight` (#580): the interval its parser already refuses below 1.
pub const RULES: app_config::schema::Rules = app_config::schema::Rules {
	required_when: &[],
	bounds: &[app_config::schema::Bound {
		variable: "HEALTH_CHECK_INTERVAL_SECS",
		min: Some(1.0),
		max: None,
		when: &[],
	}],
};

/// The OBS NATS service's whole configuration.
///
/// Each field is also a flag. A value that does not parse stops startup with
/// an error naming the variable; it never falls back to the default.
#[derive(Clone, Parser)]
#[command(author, version, about, long_about = None)]
#[command(mut_arg("rust_log", |arg| arg.default_value("some_obs=info,obs_websocket=info,some_shutdown=info")))]
pub struct Config {
	/// `NATS_URL`. The image always sets it (`infra/docker/Dockerfile.obs`).
	#[command(flatten)]
	pub nats: NatsArgs,

	/// obs-websocket's host.
	#[arg(long, env = "OBS_HOST", default_value_t = ObsConfig::default().host, help_heading = "OBS")]
	pub obs_host: String,

	/// obs-websocket's port.
	#[arg(long, env = "OBS_PORT", default_value_t = ObsConfig::default().port, help_heading = "OBS")]
	pub obs_port: u16,

	/// obs-websocket's server password. Empty means OBS has authentication off.
	#[arg(long, env = "OBS_PASSWORD", hide_env_values = true, hide_default_value = true, default_value_t = ObsConfig::default().password, help_heading = "OBS")]
	pub obs_password: String,

	/// Seconds between checks that OBS is still connected. At least 1.
	#[arg(long, env = "HEALTH_CHECK_INTERVAL_SECS", value_name = "HEALTH_CHECK_INTERVAL_SECS", default_value = "30", value_parser = parse_interval, help_heading = "OBS")]
	pub health_check_interval: Duration,

	/// Seconds the service's tasks get to finish once it is told to stop.
	#[arg(long, env = "SHUTDOWN_TIMEOUT_SECS", value_name = "SHUTDOWN_TIMEOUT_SECS", default_value = "10", value_parser = parse_seconds, help_heading = "Service")]
	pub shutdown_timeout: Duration,

	/// `RUST_LOG`, defaulting to this crate and the two it logs through.
	#[command(flatten)]
	pub log: LogArgs,
}

impl Config {
	/// Where to reach OBS.
	#[must_use]
	pub fn obs(&self) -> ObsConfig {
		ObsConfig {
			host: self.obs_host.clone(),
			port: self.obs_port,
			password: self.obs_password.clone(),
		}
	}
}

// By hand, so the password never reaches a log line through `?config`.
impl std::fmt::Debug for Config {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Config")
			.field("nats", &self.nats)
			.field("obs_host", &self.obs_host)
			.field("obs_port", &self.obs_port)
			.field("obs_password", &"***")
			.field("health_check_interval", &self.health_check_interval)
			.field("shutdown_timeout", &self.shutdown_timeout)
			.field("log", &self.log)
			.finish()
	}
}

fn parse_seconds(seconds: &str) -> Result<Duration, std::num::ParseIntError> {
	seconds.parse().map(Duration::from_secs)
}

/// A zero period would panic `tokio::time::interval` once the service is up,
/// so it is refused here instead.
fn parse_interval(seconds: &str) -> Result<Duration, Box<dyn std::error::Error + Send + Sync>> {
	let interval = parse_seconds(seconds)?;
	if interval.is_zero() {
		return Err("must be at least 1".into());
	}
	Ok(interval)
}

#[cfg(test)]
mod tests {
	use super::Config;
	use clap::{CommandFactory, Parser};

	#[test]
	fn config_definition_is_valid() {
		Config::command().debug_assert();
	}

	/// ENV1 (README.md, "Configuration"); see `app_config::env1`. Each
	/// refused one used to become its default.
	#[test]
	fn every_variable_that_does_not_parse_is_refused_by_name() {
		app_config::env1::assert_every_variable_is_refused_by_name::<Config>(
			"some-obs",
			&[
				("OBS_PORT", "not-a-port"),
				("HEALTH_CHECK_INTERVAL_SECS", "0"),
				("SHUTDOWN_TIMEOUT_SECS", "-1"),
				("RUST_LOG", "info,=[bad"),
				// Twice: a zero interval is refused by the parser, a word by its number.
				("HEALTH_CHECK_INTERVAL_SECS", "thirty"),
			],
			&["NATS_URL", "OBS_HOST", "OBS_PASSWORD"],
		);
	}

	/// CFG7 (#503): `infra/env/some-obs.json` is this `Config`.
	#[test]
	fn the_checked_in_schema_is_this_config() {
		let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infra/env/some-obs.json");
		app_config::schema::assert_checked_in("some-obs", &Config::command(), &super::RULES, &path);
		app_config::schema::assert_groups_hold_only_their_own(&Config::command());
	}

	#[test]
	fn the_password_is_not_in_debug_output() {
		let config = Config::try_parse_from(["some-obs", "--obs-password", "hunter2"]).unwrap();
		assert!(!std::fmt::format(format_args!("{config:?}")).contains("hunter2"));
	}
}

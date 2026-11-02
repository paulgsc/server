//! Everything `some-obs` reads from its environment (#484).

use app_config::{LogArgs, NatsArgs};
use clap::Parser;
use obs_websocket::ObsConfig;
use std::time::Duration;

// Constants of the service, not variables (#617): each was its variable's
// default. Changing one is a PR with its reason.

/// Time between checks that OBS is still connected.
pub const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(30);
// A zero period would panic `tokio::time::interval` once the service is up.
const _: () = assert!(!HEALTH_CHECK_INTERVAL.is_zero());

/// Time the service's tasks get to finish once it is told to stop.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

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
			.field("log", &self.log)
			.finish()
	}
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
			&[("OBS_PORT", "not-a-port"), ("RUST_LOG", "info,=[bad")],
			&["NATS_URL", "OBS_HOST", "OBS_PASSWORD"],
		);
	}

	/// CFG7 (#503): `infra/env/some-obs.json` is this `Config`.
	#[test]
	fn the_checked_in_schema_is_this_config() {
		let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infra/env/some-obs.json");
		app_config::schema::assert_checked_in("some-obs", &Config::command(), &app_config::schema::NO_RULES, &path);
		app_config::schema::assert_groups_hold_only_their_own(&Config::command());
	}

	#[test]
	fn the_password_is_not_in_debug_output() {
		let config = Config::try_parse_from(["some-obs", "--nats-url", "nats://nats:4222", "--obs-password", "hunter2"]).unwrap();
		assert!(!std::fmt::format(format_args!("{config:?}")).contains("hunter2"));
	}
}

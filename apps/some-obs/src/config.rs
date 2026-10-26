//! Everything `some-obs` reads from its environment (#484).

use clap::Parser;
use obs_websocket::ObsConfig;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

/// The OBS NATS service's whole configuration.
///
/// Each field is also a flag. A value that does not parse stops startup with
/// an error naming the variable; it never falls back to the default.
#[derive(Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Config {
	/// NATS server to connect to.
	///
	/// The default only works beside a NATS that publishes 4222 on the same
	/// machine (HOST-SPECIFIC, see `some_transport::DEV_HOST_NATS_URL`); the
	/// image always sets it (`infra/docker/Dockerfile.obs`).
	#[arg(long, env = "NATS_URL", default_value = some_transport::DEV_HOST_NATS_URL)]
	pub nats_url: String,

	/// obs-websocket's host.
	#[arg(long, env = "OBS_HOST", default_value_t = ObsConfig::default().host)]
	pub obs_host: String,

	/// obs-websocket's port.
	#[arg(long, env = "OBS_PORT", default_value_t = ObsConfig::default().port)]
	pub obs_port: u16,

	/// obs-websocket's server password. Empty means OBS has authentication off.
	#[arg(long, env = "OBS_PASSWORD", hide_env_values = true, hide_default_value = true, default_value_t = ObsConfig::default().password)]
	pub obs_password: String,

	/// Seconds between checks that OBS is still connected. At least 1.
	#[arg(long, env = "HEALTH_CHECK_INTERVAL_SECS", value_name = "HEALTH_CHECK_INTERVAL_SECS", default_value = "30", value_parser = parse_interval)]
	pub health_check_interval: Duration,

	/// Seconds the service's tasks get to finish once it is told to stop.
	#[arg(long, env = "SHUTDOWN_TIMEOUT_SECS", value_name = "SHUTDOWN_TIMEOUT_SECS", default_value = "10", value_parser = parse_seconds)]
	pub shutdown_timeout: Duration,

	/// `tracing` filter directives, e.g. `some_obs=debug`.
	#[arg(long, env = "RUST_LOG", default_value = "some_obs=info,obs_websocket=info,some_shutdown=info", value_parser = parse_filter)]
	pub rust_log: String,
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
			.field("nats_url", &self.nats_url)
			.field("obs_host", &self.obs_host)
			.field("obs_port", &self.obs_port)
			.field("obs_password", &"***")
			.field("health_check_interval", &self.health_check_interval)
			.field("shutdown_timeout", &self.shutdown_timeout)
			.field("rust_log", &self.rust_log)
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
	/// to neither list fails here. Each refused one used to become its default.
	///
	/// Clap applies the same parser to the flag and the variable, so the flag
	/// stands in for the variable without touching the process environment.
	/// Its error names the argument by its value name, never by the variable
	/// a value came from, so each variable is its own value name.
	#[test]
	fn every_variable_that_does_not_parse_is_refused_by_name() {
		const REFUSED: [(&str, &str); 5] = [
			("OBS_PORT", "not-a-port"),
			("HEALTH_CHECK_INTERVAL_SECS", "0"),
			("SHUTDOWN_TIMEOUT_SECS", "-1"),
			("RUST_LOG", "info,=[bad"),
			// Twice: a zero interval is refused by the parser, a word by its number.
			("HEALTH_CHECK_INTERVAL_SECS", "thirty"),
		];
		const FREE_TEXT: [&str; 3] = ["NATS_URL", "OBS_HOST", "OBS_PASSWORD"];
		let mut seen = Vec::new();
		for arg in Config::command().get_arguments() {
			let Some(env) = arg.get_env().and_then(std::ffi::OsStr::to_str) else { continue };
			seen.push(env.to_owned());
			let value_names: Vec<&str> = arg.get_value_names().unwrap_or_default().iter().map(clap::builder::Str::as_str).collect();
			assert_eq!(value_names, [env], "{env} is not its own value name, so its errors do not name it");
			let refused: Vec<&str> = REFUSED.iter().filter(|(name, _)| *name == env).map(|(_, bad)| *bad).collect();
			for bad in &refused {
				let flag = String::from("--") + arg.get_long().unwrap() + "=" + bad;
				let err = Config::try_parse_from(["some-obs", &flag]).unwrap_err();
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

	#[test]
	fn the_password_is_not_in_debug_output() {
		let config = Config::try_parse_from(["some-obs", "--obs-password", "hunter2"]).unwrap();
		assert!(!std::fmt::format(format_args!("{config:?}")).contains("hunter2"));
	}
}

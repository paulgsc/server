//! Where the examples find OBS: the variables `some-obs` reads, through clap,
//! with `ObsConfig::default()`'s values when unset. A value that does not
//! parse stops the example, naming the variable (#484).

use obs_websocket::ObsConfig;

#[derive(clap::Args)]
pub(crate) struct ObsArgs {
	/// obs-websocket's host.
	#[arg(long = "obs-host", env = "OBS_HOST", value_name = "OBS_HOST", default_value_t = ObsConfig::default().host)]
	host: String,

	/// obs-websocket's port.
	#[arg(long = "obs-port", env = "OBS_PORT", value_name = "OBS_PORT", default_value_t = ObsConfig::default().port)]
	port: u16,

	/// obs-websocket's server password. Empty means OBS has authentication off.
	#[arg(long = "obs-password", env = "OBS_PASSWORD", value_name = "OBS_PASSWORD", hide_env_values = true, hide_default_value = true, default_value_t = ObsConfig::default().password)]
	password: String,
}

impl ObsArgs {
	pub(crate) fn into_config(self) -> ObsConfig {
		ObsConfig {
			host: self.host,
			port: self.port,
			password: self.password,
		}
	}
}

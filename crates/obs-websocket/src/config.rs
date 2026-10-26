use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Where to reach obs-websocket and how to authenticate.
///
/// The caller fills it in: this crate reads no environment (#484). `some-obs`
/// reads `OBS_HOST`, `OBS_PORT` and `OBS_PASSWORD` into it through clap, so a
/// port that does not parse stops it at start instead of becoming the default.
#[derive(Clone, Serialize, Deserialize)]
pub struct ObsConfig {
	pub host: String,
	pub port: u16,
	/// obs-websocket's server password. Empty means OBS has authentication off.
	pub password: String,
}

/// `OBS_HOST`'s, `OBS_PORT`'s and `OBS_PASSWORD`'s defaults in `some-obs` and
/// the examples. HOST-SPECIFIC (#406): the host is a machine on the dev host's
/// LAN; the image sets `OBS_HOST` (`infra/docker/Dockerfile.obs`).
impl Default for ObsConfig {
	fn default() -> Self {
		Self {
			host: "10.0.0.25".to_string(),
			port: 4455,
			password: "pwd".to_string(),
		}
	}
}

impl std::fmt::Debug for ObsConfig {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ObsConfig")
			.field("host", &self.host)
			.field("port", &self.port)
			.field("password", &"***")
			.finish()
	}
}

/// Reconnect pacing for callers that re-run [`crate::ObsWebSocketManager::connect`]
/// after a dropped connection.
#[derive(Debug, Clone)]
pub struct RetryConfig {
	pub initial_delay: Duration,
	pub max_delay: Duration,
}

impl Default for RetryConfig {
	fn default() -> Self {
		Self {
			initial_delay: Duration::from_secs(1),
			max_delay: Duration::from_secs(60),
		}
	}
}

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Where to reach obs-websocket and how to authenticate.
#[derive(Clone, Serialize, Deserialize)]
pub struct ObsConfig {
	pub host: String,
	pub port: u16,
	/// obs-websocket's server password. Empty means OBS has authentication off.
	pub password: String,
}

impl ObsConfig {
	/// Reads `OBS_HOST`, `OBS_PORT` and `OBS_PASSWORD`, falling back to
	/// [`Self::default`] for any that are unset or, for the port, unparsable.
	#[must_use]
	pub fn from_env() -> Self {
		let defaults = Self::default();
		Self {
			host: std::env::var("OBS_HOST").unwrap_or(defaults.host),
			port: std::env::var("OBS_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(defaults.port),
			password: std::env::var("OBS_PASSWORD").unwrap_or(defaults.password),
		}
	}
}

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

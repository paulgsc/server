use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Complete configuration for the OBS NATS service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
	/// NATS server URL
	pub nats_url: String,
	/// Health check interval
	pub health_check_interval: Duration,
	/// Graceful shutdown timeout
	pub shutdown_timeout: Duration,
}

impl Config {
	/// Load configuration from environment variables with sensible defaults
	pub fn from_env() -> Result<Self> {
		Ok(Self {
			nats_url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string()),
			health_check_interval: Duration::from_secs(std::env::var("HEALTH_CHECK_INTERVAL_SECS").ok().and_then(|i| i.parse().ok()).unwrap_or(30)),
			shutdown_timeout: Duration::from_secs(std::env::var("SHUTDOWN_TIMEOUT_SECS").ok().and_then(|t| t.parse().ok()).unwrap_or(10)),
		})
	}
}

impl Default for Config {
	fn default() -> Self {
		Self {
			nats_url: "nats://localhost:4222".to_string(),
			health_check_interval: Duration::from_secs(30),
			shutdown_timeout: Duration::from_secs(10),
		}
	}
}

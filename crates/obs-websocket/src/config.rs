use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsConfig {
	pub host: String,
	pub port: u16,
	pub password: String,
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

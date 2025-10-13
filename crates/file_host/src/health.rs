use crate::Config;
use anyhow::Result;

pub async fn perform_health_check(config: &Config) -> Result<()> {
	use std::process;

	let url = format!("http://{}:{}/health", config.health_check_host, config.health_check_port);

	match reqwest::Client::new().get(&url).timeout(std::time::Duration::from_secs(10)).send().await {
		Ok(response) => {
			if response.status().is_success() {
				println!("Health check passed");
				process::exit(0);
			} else {
				eprintln!("Health check failed: HTTP {}", response.status());
				process::exit(1);
			}
		}
		Err(e) => {
			eprintln!("Health check failed: {}", e);
			process::exit(1);
		}
	}
}

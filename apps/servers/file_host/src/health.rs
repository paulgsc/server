use crate::Config;
use anyhow::Result;

/// The `--health-check` CLI flag's target, invoked by `infra/compose/file_host.yml`'s
/// container healthcheck (`file_host --health-check`). Deliberately `/ready`,
/// not `/health` — see #213 (F1). This is what `depends_on: condition:
/// service_healthy` gates on elsewhere in the compose stack, so it needs to
/// mean "can serve", not merely "the process answers a socket". The flag
/// keeps its name (external scripts and this compose file already invoke it)
/// even though its target moved; only the request behind it changed.
pub async fn perform_health_check(config: &Config) -> Result<()> {
	use std::process;

	let url = format!("http://{}:{}/ready", config.health_check_host, config.health_check_port);

	match reqwest::Client::new().get(&url).timeout(std::time::Duration::from_secs(10)).send().await {
		Ok(response) => {
			if response.status().is_success() {
				println!("Readiness check passed");
				process::exit(0);
			} else {
				eprintln!("Readiness check failed: HTTP {}", response.status());
				process::exit(1);
			}
		}
		Err(e) => {
			eprintln!("Readiness check failed: {}", e);
			process::exit(1);
		}
	}
}

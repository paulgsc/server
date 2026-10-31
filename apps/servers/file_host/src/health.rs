//! The `--health-check` CLI flag, invoked by `infra/compose/file_host.yml`'s
//! container healthcheck (`file_host --health-check`).
//!
//! It asks `/ready`, not `/health` (#213, F1): "healthy" has to mean the
//! process can serve, not merely that it answers a socket. But it passes on
//! a **degraded** answer too (`handlers::readiness::Verdict`), and fails
//! only on **unavailable**: a critical dependency down. A database behind
//! this build is degraded, so it no longer marks the container unhealthy.
//! The flag keeps its name, since external scripts and the compose file
//! already invoke it.
//!
//! It asks over plain HTTP, since `file_host` serves no TLS, so it asks a
//! loopback address only: the probe is of this container's own process,
//! and a `--health-check-host` anywhere else is refused, not sent over the
//! network in the clear.

use crate::handlers::readiness::Verdict;
use crate::Config;
use anyhow::Result;
use reqwest::Url;
use serde::Deserialize;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr};

/// The part of `/ready`'s body this reads.
#[derive(Deserialize)]
struct Answer {
	status: Verdict,
	#[serde(default)]
	dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
	name: String,
	healthy: bool,
	#[serde(default)]
	error: Option<String>,
}

/// What the healthcheck makes of `/ready`'s status code and body: pass, with
/// what is down when degraded, or fail, with why.
fn judge(code: u16, body: &str) -> Result<String, String> {
	if code == 200 {
		return Ok("Readiness check passed".to_string());
	}
	let answer: Answer =
		serde_json::from_str(body).map_err(|e| "Readiness check failed: HTTP ".to_string() + &code.to_string() + ", and its body did not read: " + &e.to_string())?;
	let down = answer
		.dependencies
		.iter()
		.filter(|d| !d.healthy)
		.map(|d| d.name.clone() + ": " + d.error.as_deref().unwrap_or("down"))
		.collect::<Vec<_>>()
		.join("; ");
	match answer.status {
		Verdict::Degraded => Ok("Readiness check passed, degraded: ".to_string() + &down),
		Verdict::Unavailable => Err("Readiness check failed, unavailable: ".to_string() + &down),
		Verdict::Ready => Err("Readiness check failed: HTTP ".to_string() + &code.to_string() + " with every dependency ready"),
	}
}

/// `/ready` on `host:port`, when `host` is a loopback address (`localhost`
/// or an IP that is one); refused otherwise.
fn ready_url(host: &str, port: u16) -> Result<Url, String> {
	let ip: IpAddr = if host.eq_ignore_ascii_case("localhost") {
		IpAddr::V4(Ipv4Addr::LOCALHOST)
	} else {
		host
			.parse()
			.map_err(|_| "Readiness check refused: --health-check-host must be a loopback address, not ".to_string() + host)?
	};
	if !ip.is_loopback() {
		return Err("Readiness check refused: --health-check-host must be a loopback address, not ".to_string() + host);
	}
	let mut url = Url::parse("http://127.0.0.1/ready").map_err(|e| e.to_string())?;
	url.set_ip_host(ip).map_err(|()| "Readiness check refused: no URL for ".to_string() + host)?;
	url
		.set_port(Some(port))
		.map_err(|()| "Readiness check refused: no URL for port ".to_string() + &port.to_string())?;
	Ok(url)
}

/// Exits 0 when `/ready` says ready or degraded, 1 otherwise, saying why on
/// the way out: Docker keeps the line in `docker inspect`'s health log.
///
/// # Errors
/// Never returns: it always exits. The `Result` is `main`'s.
pub async fn perform_health_check(config: &Config) -> Result<()> {
	let verdict = match ready_url(&config.health_check_host, config.health_check_port) {
		Ok(url) => match reqwest::Client::new().get(url).timeout(std::time::Duration::from_secs(10)).send().await {
			Ok(response) => {
				let code = response.status().as_u16();
				let body = response.text().await.unwrap_or_default();
				judge(code, &body)
			}
			Err(e) => Err("Readiness check failed: ".to_string() + &e.to_string()),
		},
		Err(refused) => Err(refused),
	};

	match verdict {
		Ok(line) => {
			let _ = writeln!(io::stdout(), "{line}");
			std::process::exit(0);
		}
		Err(line) => {
			let _ = writeln!(io::stderr(), "{line}");
			std::process::exit(1);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const BEHIND: &str = r#"{"ready":false,"status":"degraded","dependencies":[
		{"name":"sqlite","healthy":true,"critical":true},
		{"name":"schema","healthy":false,"critical":false,"error":"5 pending (20261029000200 store_each_game_feed_body_once); run `migrate`"}]}"#;

	#[test]
	fn ready_passes() {
		assert!(judge(200, "").is_ok());
	}

	#[test]
	fn a_database_behind_this_build_passes_and_names_what_is_down() {
		let line = judge(503, BEHIND).unwrap();
		assert!(line.contains("degraded") && line.contains("schema: 5 pending"), "{line}");
	}

	#[test]
	fn a_critical_dependency_down_fails() {
		let body = r#"{"ready":false,"status":"unavailable","dependencies":[{"name":"sqlite","healthy":false,"critical":true,"error":"disk I/O error"}]}"#;
		let line = judge(503, body).unwrap_err();
		assert!(line.contains("unavailable") && line.contains("sqlite: disk I/O error"), "{line}");
	}

	#[test]
	fn an_answer_it_cannot_read_fails() {
		// A proxy's error page, or a build from before the three levels.
		assert!(judge(502, "<html>Bad Gateway</html>").is_err());
		assert!(judge(503, r#"{"ready":false}"#).is_err());
	}

	#[test]
	fn it_asks_a_loopback_address_only() {
		assert_eq!(ready_url("127.0.0.1", 3000).unwrap().as_str(), "http://127.0.0.1:3000/ready");
		assert_eq!(ready_url("localhost", 3000).unwrap().as_str(), "http://127.0.0.1:3000/ready");
		assert_eq!(ready_url("::1", 8080).unwrap().as_str(), "http://[::1]:8080/ready");
		for elsewhere in ["10.0.0.5", "file-host.home.maishatu.com", "0.0.0.0", ""] {
			assert!(ready_url(elsewhere, 3000).is_err(), "{elsewhere}");
		}
	}

	#[test]
	fn a_non_200_that_claims_ready_fails() {
		assert!(judge(503, r#"{"status":"ready"}"#).is_err());
	}
}

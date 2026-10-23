//! How much of the Jobs mailbox's free D1 allowance today has used, asked
//! of Cloudflare and published as Prometheus gauges for the MAILBOX
//! dashboard (`infra/grafana/dashboards/mailbox.jsonnet`).
//!
//! The mailbox's Cloudflare account has no payment method, on purpose: past
//! 5,000,000 rows read or 100,000 rows written in a UTC day, every query
//! fails until 00:00 UTC (the Worker answers `503`), and nothing is billed.
//! Today's load is tens of rows a day; this exists to show the trend and to
//! catch a runaway loop before the limit does.
//!
//! **Why this is a poll.** Cloudflare does not push analytics, so home asks,
//! every `CF_USAGE_SECONDS` (300 by default): one GraphQL request for today's
//! sums and the database's size. Cloudflare's datasets run a few minutes
//! behind, so asking more often shows nothing newer.
//!
//! What this module promises, as docs/identity.md invariant 20 states:
//!
//! - It makes one kind of request, a read of one database's analytics, with
//!   a read-only token (`CF_API_TOKEN`) that is not the mailbox's own, and
//!   writes nothing anywhere but gauges.
//! - The token and the account id never reach a log line: the token is a
//!   `mailbox_kit::Token`, errors are logged by kind.
//! - A failed read never looks like a quiet day: the gauges keep their last
//!   value, `d1_usage_failing{error}` says why, and
//!   `d1_usage_last_success_timestamp_seconds` says since when.

use crate::{AppState, Config};
use chrono::Utc;
use mailbox_kit::usage::Analytics;
use mailbox_kit::{ReqwestTransport, Token};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// The shortest interval `CF_USAGE_SECONDS` may set.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// What the collector needs, or `None` when the deployment has not turned it on.
#[derive(Clone)]
pub struct UsageContext {
	pub analytics: Arc<Analytics<ReqwestTransport>>,
	pub interval: Duration,
}

impl UsageContext {
	/// Validate the configuration once, at startup: off is fine and logged;
	/// on with anything missing is a hard error naming the variable, never
	/// its value.
	///
	/// # Errors
	/// `CF_USAGE_ENABLED` with `CF_ACCOUNT_ID`, `CF_API_TOKEN` or
	/// `CF_D1_DATABASE_ID` unset or empty, or `CF_USAGE_SECONDS` below 60.
	pub fn from_config(config: &Config) -> anyhow::Result<Option<Self>> {
		if !config.cf_usage_enabled {
			info!("the mailbox's D1 usage is not read; the MAILBOX dashboard's usage row will read OFF");
			return Ok(None);
		}
		let set = |value: Option<&String>| value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
		let Some(account) = set(config.cf_account_id.as_ref()) else {
			anyhow::bail!("CF_USAGE_ENABLED is set but CF_ACCOUNT_ID is not");
		};
		let Some(token) = set(config.cf_api_token.as_ref()) else {
			anyhow::bail!("CF_USAGE_ENABLED is set but CF_API_TOKEN is not");
		};
		let Some(database) = set(config.cf_d1_database_id.as_ref()) else {
			anyhow::bail!("CF_USAGE_ENABLED is set but CF_D1_DATABASE_ID is not");
		};
		let interval = Duration::from_secs(config.cf_usage_seconds);
		if interval < MIN_INTERVAL {
			anyhow::bail!("CF_USAGE_SECONDS must be at least 60");
		}
		let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
		let analytics = Analytics::new(account, database, Token::new(token), ReqwestTransport::new(client));
		Ok(Some(Self {
			analytics: Arc::new(analytics),
			interval,
		}))
	}
}

/// Start the collector if the deployment configured one, and say which on
/// `d1_usage_enabled` either way.
pub fn start(state: &AppState) {
	crate::metrics::d1_usage::record_enabled(state.d1_usage.is_some());
	let Some(context) = state.d1_usage.clone() else { return };
	let cancel = state.core.cancel_token.clone();
	crate::metrics::d1_usage::record_interval(context.interval);
	tokio::spawn(async move {
		info!(interval_secs = context.interval.as_secs(), "D1 usage collector started");
		let mut ticker = tokio::time::interval(context.interval);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("D1 usage collector cancelled");
					return;
				}
				_ = ticker.tick() => {
					crate::metrics::d1_usage::record_attempt();
					// The free plan's day is the UTC day.
					let day = Utc::now().date_naive().to_string();
					match context.analytics.day(&day).await {
						Ok(usage) => crate::metrics::d1_usage::record_usage(&usage),
						Err(err) => {
							warn!(error = %err, class = err.kind(), "D1 usage read failed");
							crate::metrics::d1_usage::record_failure(err.kind());
						}
					}
				}
			}
		}
	});
}

#[cfg(test)]
mod tests {
	use super::UsageContext;
	use crate::Config;
	use clap::Parser;

	const REQUIRED: [&str; 9] = [
		"file_host",
		"--obs-host",
		"h",
		"--obs-password",
		"p",
		"--database-url",
		"sqlite::memory:",
		"--auth-enabled",
		"false",
	];

	fn context(extra: &[&str]) -> anyhow::Result<Option<UsageContext>> {
		UsageContext::from_config(&Config::try_parse_from(REQUIRED.iter().chain(extra).copied()).unwrap())
	}

	#[test]
	fn off_unless_enabled_and_never_half_configured() {
		assert!(context(&[]).unwrap().is_none(), "off by default");
		let on = ["--cf-usage-enabled", "true"];
		let account = ["--cf-account-id", "account-id-secret"];
		let token = ["--cf-api-token", "analytics-token-secret"];
		let database = ["--cf-d1-database-id", "db-id"];
		for (missing, args) in [
			("CF_ACCOUNT_ID", [&on[..], &token[..], &database[..]].concat()),
			("CF_API_TOKEN", [&on[..], &account[..], &database[..]].concat()),
			("CF_D1_DATABASE_ID", [&on[..], &account[..], &token[..]].concat()),
		] {
			let err = context(&args).err().unwrap().to_string();
			assert!(err.contains(missing), "{err}");
			assert!(!err.contains("secret"), "no value in the error: {err}");
		}
		let all = [&on[..], &account[..], &token[..], &database[..]].concat();
		assert!(context(&[&all[..], &["--cf-usage-seconds", "59"][..]].concat()).is_err(), "at least a minute");
		assert_eq!(context(&all).unwrap().unwrap().interval.as_secs(), 300);
	}
}

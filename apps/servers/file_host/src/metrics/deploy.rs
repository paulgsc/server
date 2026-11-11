//! Home's on-demand deploy (`crate::deploy`), made loud and measurable.
//!
//! What clients asked `/compat`, what home's deploy did (read from its
//! `state.json` on `metrics::periodic`'s sweep), a marker nobody acted on,
//! and the migration backups held (`crate::migrate`).
//!
//! The deploy runs outside this process (`envctl deploy --on-demand`, from a systemd
//! path unit), so its totals are its own file's, exported here as gauges
//! that only rise: Prometheus' `increase()` reads them as counters, and a
//! rewrite of the file (a reset) reads as one.
//! `infra/prometheus/rules/home.yml`'s `home-deploy` group alerts on them;
//! the Deploy row on the storage dashboard draws them.

use crate::deploy::{Deploy, Recorded};

/// Every result `/compat` answers with, at 0 from the first sweep, so a
/// result never asked yet reads as zero rather than a missing series.
const RESULTS: [Deploy; 8] = [
	Deploy::None,
	Deploy::ClientBehind,
	Deploy::Refused,
	Deploy::InFlight,
	Deploy::Unavailable,
	Deploy::Cooldown,
	Deploy::Wanted,
	Deploy::CannotRequest,
];

/// One `/compat` ask, by what it was answered.
pub fn record_compat(result: Deploy) {
	metrics::counter!("compat_checks_total", "result" => result.label()).increment(1);
}

/// Every result's series at 0, and this build's epochs.
pub fn register_all() {
	for result in RESULTS {
		metrics::counter!("compat_checks_total", "result" => result.label()).increment(0);
	}
	metrics::gauge!("contract_epoch").set(f64::from(crate::contract::CONTRACT_EPOCH));
	metrics::gauge!("contract_oldest_supported").set(f64::from(crate::contract::OLDEST_SUPPORTED));
}

#[allow(clippy::cast_precision_loss)] // counts and byte totals stay far inside f64's exact range
const fn as_f64(value: u64) -> f64 {
	value as f64
}

#[allow(clippy::cast_precision_loss)] // seconds since the epoch stay far inside f64's exact range
const fn seconds(value: i64) -> f64 {
	value as f64
}

/// What home's deploy recorded, at `now` (seconds since the epoch), with
/// the marker's time if one is waiting and the migration backups held.
pub fn record_state(recorded: &Recorded, marker_since: Option<i64>, now: i64, backups: (u64, u64)) {
	let counts = &recorded.counts;
	for (outcome, value) in [
		("deployed", counts.deployed),
		("rolled_back", counts.rolled_back),
		("failed", counts.failed),
		("no_newer_image", counts.no_newer_image),
		("deferred", counts.deferred),
	] {
		metrics::gauge!("home_deploy_runs", "outcome" => outcome).set(as_f64(value));
	}
	metrics::gauge!("home_deploy_pulls").set(as_f64(counts.pulls));
	metrics::gauge!("home_deploy_pulled_bytes").set(as_f64(counts.pulled_bytes));
	if let Some(last) = &recorded.last_run {
		metrics::gauge!("home_deploy_last_run_timestamp_seconds").set(seconds(last.at));
		metrics::gauge!("home_deploy_last_exit_code").set(f64::from(last.exit));
	}
	if let Some(latest) = recorded.latest_seen_epoch {
		metrics::gauge!("home_deploy_latest_seen_epoch").set(f64::from(latest));
	}
	let waiting = marker_since.map_or(0, |since| now.saturating_sub(since).max(0));
	metrics::gauge!("home_deploy_wanted_age_seconds").set(seconds(waiting));
	metrics::gauge!("migration_backups").set(as_f64(backups.0));
	metrics::gauge!("migration_backup_bytes").set(as_f64(backups.1));
}

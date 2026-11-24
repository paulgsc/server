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
///
/// `recorded` is an error when `state.json` is there and can't be read, and
/// `marker_since` an error when the marker can't be read (`Ok(None)`: no
/// marker). Either unknown is `NaN`, a gap on the graph, never 0 (HD3,
/// #1208): a total read as 0 and then as itself again would be a reset and
/// an increase `DeployFailed` reads as a failure. `DeployFailed` drops the
/// `NaN` samples before `increase()`, so the gap is absent to it and the
/// totals the healing deploy writes anew still read as a reset it sees.
///
/// This sweep is the pass that looks, so it holds each unreadable file open
/// at `home-deploy`'s `read_state` and `read_marker` (docs/home-compute.md,
/// invariant 9: a level, never a recount) and logs why at `warn`.
/// `HomeFailuresOpen` tells the owner while one stays unreadable; `/compat`
/// decides on it without saying it again on every ask.
pub fn record_state(recorded: &std::io::Result<Recorded>, marker_since: &std::io::Result<Option<i64>>, now: i64, backups: (u64, u64)) {
	if let Err(err) = recorded {
		tracing::warn!(error = %err, "home's deploy record can't be read: its gauges are unknown");
	}
	if let Err(err) = marker_since {
		tracing::warn!(error = %err, "home's deploy marker can't be read: its age is unknown");
	}
	crate::failure::open(crate::failure::Step::HomeDeployReadState, usize::from(recorded.is_err()));
	crate::failure::open(crate::failure::Step::HomeDeployReadMarker, usize::from(marker_since.is_err()));
	let counts = recorded.as_ref().map(|recorded| &recorded.counts);
	let total = |value: fn(&crate::deploy::Counts) -> u64| counts.as_ref().map_or(f64::NAN, |counts| as_f64(value(counts)));
	for (outcome, value) in [
		("deployed", total(|counts| counts.deployed)),
		("rolled_back", total(|counts| counts.rolled_back)),
		("failed", total(|counts| counts.failed)),
		("no_newer_image", total(|counts| counts.no_newer_image)),
		("deferred", total(|counts| counts.deferred)),
	] {
		metrics::gauge!("home_deploy_runs", "outcome" => outcome).set(value);
	}
	metrics::gauge!("home_deploy_pulls").set(total(|counts| counts.pulls));
	metrics::gauge!("home_deploy_pulled_bytes").set(total(|counts| counts.pulled_bytes));
	recorded.as_ref().map_or_else(
		|_| {
			metrics::gauge!("home_deploy_last_run_timestamp_seconds").set(f64::NAN);
			metrics::gauge!("home_deploy_last_exit_code").set(f64::NAN);
			metrics::gauge!("home_deploy_latest_seen_epoch").set(f64::NAN);
		},
		|recorded| {
			if let Some(last) = &recorded.last_run {
				metrics::gauge!("home_deploy_last_run_timestamp_seconds").set(seconds(last.at));
				metrics::gauge!("home_deploy_last_exit_code").set(f64::from(last.exit));
			}
			if let Some(latest) = recorded.latest_seen_epoch {
				metrics::gauge!("home_deploy_latest_seen_epoch").set(f64::from(latest));
			}
		},
	);
	let waiting = marker_since
		.as_ref()
		.map_or(f64::NAN, |since| seconds(since.map_or(0, |since| now.saturating_sub(since).max(0))));
	metrics::gauge!("home_deploy_wanted_age_seconds").set(waiting);
	metrics::gauge!("migration_backups").set(as_f64(backups.0));
	metrics::gauge!("migration_backup_bytes").set(as_f64(backups.1));
}

#[cfg(test)]
mod tests {
	use super::record_state;
	use crate::deploy::{Counts, Recorded};
	use metrics_util::debugging::{DebugValue, DebuggingRecorder};

	/// Every gauge `record_state` set, by name and `outcome` or `step` label.
	fn gauges(state: &std::io::Result<Recorded>, marker: &std::io::Result<Option<i64>>) -> Vec<(String, f64)> {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		metrics::with_local_recorder(&recorder, || record_state(state, marker, 1_000, (0, 0)));
		snapshotter
			.snapshot()
			.into_vec()
			.into_iter()
			.filter_map(|(key, _, _, value)| {
				let key = key.key();
				let outcome = key
					.labels()
					.find(|l| l.key() == "outcome" || l.key() == "step")
					.map(|l| String::from("/") + l.value())
					.unwrap_or_default();
				match value {
					DebugValue::Gauge(value) => Some((key.name().to_owned() + &outcome, value.into_inner())),
					_ => None,
				}
			})
			.filter(|(name, _)| name.starts_with("home_deploy") || name.starts_with("failures_open"))
			.collect()
	}

	/// The `failures_open` level at `step`.
	fn open(gauges: &[(String, f64)], step: &str) -> Option<f64> {
		gauges.iter().find(|(name, _)| *name == format!("failures_open/{step}")).map(|(_, value)| *value)
	}

	/// HD3 (#1208): a record that can't be read is a gap on every deploy
	/// gauge, and a marker that can't be read a gap on its age, never 0, so
	/// neither a reset `DeployFailed` reads as failures nor an age
	/// `DeployStuck` can't see.
	#[test]
	fn an_unreadable_record_or_marker_is_a_gap_never_zero() {
		let unreadable = gauges(&Err(std::io::Error::other("garbled")), &Err(std::io::Error::other("unreadable")));
		assert!(unreadable.len() >= 10, "{unreadable:?}");
		for (name, value) in unreadable.iter().filter(|(name, _)| name.starts_with("home_deploy")) {
			assert!(value.is_nan(), "{name} = {value}");
		}

		let recorded = Recorded {
			counts: Counts {
				rolled_back: 2,
				..Counts::default()
			},
			..Recorded::default()
		};
		let readable = gauges(&Ok(recorded.clone()), &Ok(None));
		let value = |name: &str| readable.iter().find(|(n, _)| n == name).map(|(_, v)| *v);
		assert_eq!(value("home_deploy_runs/rolled_back"), Some(2.0));
		assert_eq!(value("home_deploy_runs/failed"), Some(0.0));
		assert_eq!(value("home_deploy_wanted_age_seconds"), Some(0.0), "no marker is an age of 0");
		assert_eq!(
			gauges(&Ok(recorded.clone()), &Ok(Some(400)))
				.iter()
				.find(|(n, _)| n == "home_deploy_wanted_age_seconds")
				.map(|(_, v)| *v),
			Some(600.0)
		);
	}

	/// HD3 (#1208), invariant 9: each unreadable file is held open, as a
	/// level set afresh by each sweep, and a sweep that reads it clears it.
	/// Family: record unreadable, marker unreadable, both, neither.
	#[test]
	fn an_unreadable_record_or_marker_is_held_open_until_read() {
		let garbled = || Err(std::io::Error::other("garbled"));
		for (recorded, marker, state_open, marker_open) in [
			(garbled(), Ok(None), 1.0, 0.0),
			(Ok(Recorded::default()), garbled(), 0.0, 1.0),
			(garbled(), garbled(), 1.0, 1.0),
			(Ok(Recorded::default()), Ok(Some(400)), 0.0, 0.0),
		] {
			let set = gauges(&recorded, &marker);
			assert_eq!(open(&set, "read_state"), Some(state_open), "{set:?}");
			assert_eq!(open(&set, "read_marker"), Some(marker_open), "{set:?}");
		}
	}
}

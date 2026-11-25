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

/// What the gauge sweep remembers between ticks.
///
/// The deploy's totals as it last read them, and whether each file was
/// unreadable at the last tick. Held by the sweep's task, in memory only,
/// so a restart starts it empty.
#[derive(Debug, Default)]
pub struct Last {
	totals: Option<crate::deploy::Counts>,
	record_unreadable: bool,
	marker_unreadable: bool,
}

/// What home's deploy recorded, at `now` (seconds since the epoch), with
/// the marker's time if one is waiting and the migration backups held.
///
/// `recorded` is an error when `state.json` is there and can't be read, and
/// `marker_since` an error when the marker can't be read (`Ok(None)`: no
/// marker). Unknown is never 0 (HD3, #1208): a total read as 0 and then as
/// itself again would be a reset and an increase `DeployFailed` reads as a
/// failure. So while the record can't be read:
///
/// - the run totals stay as `last` last read them, since a running total that
///   was read is still true as a lower bound, and the deploy that rewrites
///   the record anew (envctl starts it from nothing) then reads as a reset
///   `DeployFailed` sees, however long it was unreadable (the
///   coordinator's call C1511, after `NaN` on the totals hid that reset
///   from `increase()`). Two bounds, where `DeployFailed` does not tell the
///   healing run's failure: its outcome's held total is exactly 1, so it
///   reads 1 then 1; or `file_host` restarted while the record was
///   unreadable, which leaves the totals absent until a tick reads them,
///   more than an hour before the healing run. `HomeFailuresOpen` tells
///   the unreadable record only once it has stayed so about 2 hours, so
///   either is told by neither alert when the record was unreadable for
///   less than that: a held total of 1 healed by a run that fails, or a
///   record unreadable for 1 to 2 hours across a restart, healed by a run
///   that fails more than an hour after the restart. Both come from envctl
///   rewriting an unreadable record from nothing (#1224);
/// - the last run's time and exit code and the newest epoch seen, which
///   state a current value, are `NaN`, a gap, as is the marker's age when it
///   can't be read.
///
/// This sweep is the pass that looks, so it holds each unreadable file open
/// at `home-deploy`'s `read_state` and `read_marker` (docs/home-compute.md,
/// invariant 9: a level, never a recount) and logs why at `warn` when a
/// file turns unreadable, not on every tick (the sweep runs every few
/// seconds). `HomeFailuresOpen` tells the owner while one stays unreadable;
/// `/compat` decides on it without saying it again on every ask.
pub fn record_state(last: &mut Last, recorded: &std::io::Result<Recorded>, marker_since: &std::io::Result<Option<i64>>, now: i64, backups: (u64, u64)) {
	if let (Err(err), false) = (recorded, last.record_unreadable) {
		tracing::warn!(error = %err, "home's deploy record can't be read: its totals stay as last read");
	}
	if let (Err(err), false) = (marker_since, last.marker_unreadable) {
		tracing::warn!(error = %err, "home's deploy marker can't be read: its age is unknown");
	}
	last.record_unreadable = recorded.is_err();
	last.marker_unreadable = marker_since.is_err();
	crate::failure::open(crate::failure::Step::HomeDeployReadState, usize::from(last.record_unreadable));
	crate::failure::open(crate::failure::Step::HomeDeployReadMarker, usize::from(last.marker_unreadable));
	if let Ok(recorded) = recorded {
		last.totals = Some(recorded.counts.clone());
	}
	if let Some(counts) = &last.totals {
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
	}
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
	use super::{record_state, Last};
	use crate::deploy::{Counts, Recorded};
	use metrics_util::debugging::{DebugValue, DebuggingRecorder};

	/// Every gauge `record_state` set, by name and `outcome` or `step` label.
	fn gauges(last: &mut Last, state: &std::io::Result<Recorded>, marker: &std::io::Result<Option<i64>>) -> Vec<(String, f64)> {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		metrics::with_local_recorder(&recorder, || record_state(last, state, marker, 1_000, (0, 0)));
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
		gauges.iter().find(|(name, _)| name.strip_prefix("failures_open/") == Some(step)).map(|(_, value)| *value)
	}

	fn value(gauges: &[(String, f64)], name: &str) -> Option<f64> {
		gauges.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
	}

	fn garbled<T>() -> std::io::Result<T> {
		Err(std::io::Error::other("garbled"))
	}

	/// HD3 (#1208), C1511: while the record can't be read its run totals
	/// stay as last read (a lower bound still true), so the deploy that
	/// writes it anew reads as a reset `DeployFailed` sees; read from no
	/// earlier tick (a restart) they are absent, never 0. What states a
	/// current value (the last run, the newest epoch, the marker's age) is a
	/// gap. Family: unreadable from the start, read then unreadable, then
	/// read anew; the marker absent, waiting, unreadable.
	#[test]
	fn an_unreadable_record_holds_its_totals_and_gaps_the_rest_never_zero() {
		let mut last = Last::default();
		let first = gauges(&mut last, &garbled(), &garbled());
		assert_eq!(value(&first, "home_deploy_runs/rolled_back"), None, "nothing read since the start: absent");
		assert_eq!(value(&first, "home_deploy_pulls"), None);
		for name in [
			"home_deploy_last_run_timestamp_seconds",
			"home_deploy_last_exit_code",
			"home_deploy_latest_seen_epoch",
			"home_deploy_wanted_age_seconds",
		] {
			assert!(value(&first, name).is_some_and(f64::is_nan), "{name}: {first:?}");
		}

		let recorded = |rolled_back| Recorded {
			counts: Counts {
				rolled_back,
				pulls: 4,
				..Counts::default()
			},
			..Recorded::default()
		};
		let read = gauges(&mut last, &Ok(recorded(3)), &Ok(None));
		assert_eq!(value(&read, "home_deploy_runs/rolled_back"), Some(3.0));
		assert_eq!(value(&read, "home_deploy_runs/failed"), Some(0.0));
		assert_eq!(value(&read, "home_deploy_wanted_age_seconds"), Some(0.0), "no marker is an age of 0");

		let held = gauges(&mut last, &garbled(), &Ok(Some(400)));
		assert_eq!(value(&held, "home_deploy_runs/rolled_back"), Some(3.0), "held as last read");
		assert_eq!(value(&held, "home_deploy_pulls"), Some(4.0));
		assert!(value(&held, "home_deploy_latest_seen_epoch").is_some_and(f64::is_nan));
		assert_eq!(value(&held, "home_deploy_wanted_age_seconds"), Some(600.0));

		let healed = gauges(&mut last, &Ok(recorded(1)), &Ok(None));
		assert_eq!(value(&healed, "home_deploy_runs/rolled_back"), Some(1.0), "written anew: a reset, then its own outcome");
	}

	/// HD3 (#1208), invariant 9: each unreadable file is held open, as a
	/// level set afresh by each sweep, and a sweep that reads it clears it.
	/// Family: record unreadable, marker unreadable, both, neither.
	#[test]
	fn an_unreadable_record_or_marker_is_held_open_until_read() {
		for (recorded, marker, state_open, marker_open) in [
			(garbled(), Ok(None), 1.0, 0.0),
			(Ok(Recorded::default()), garbled(), 0.0, 1.0),
			(garbled(), garbled(), 1.0, 1.0),
			(Ok(Recorded::default()), Ok(Some(400)), 0.0, 0.0),
		] {
			let set = gauges(&mut Last::default(), &recorded, &marker);
			assert_eq!(open(&set, "read_state"), Some(state_open), "{set:?}");
			assert_eq!(open(&set, "read_marker"), Some(marker_open), "{set:?}");
		}
	}

	/// Invariant 9: an unreadable file is said at `warn` when it turns
	/// unreadable, not on every tick, and again when it turns unreadable
	/// after a tick read it.
	#[test]
	fn an_unreadable_file_is_said_when_it_turns_unreadable_not_each_tick() {
		let (captured, _guard) = crate::privacy::capture_where(|meta| *meta.level() <= tracing::Level::WARN);
		let mut last = Last::default();
		let said = |captured: &crate::privacy::Captured, what: &str| captured.lines().iter().filter(|line| line.contains(what)).count();
		let (record, marker) = ("deploy record can't be read", "deploy marker can't be read");
		record_state(&mut last, &garbled(), &garbled(), 1_000, (0, 0));
		assert_eq!((said(&captured, record), said(&captured, marker)), (1, 1), "the first unreadable tick");
		record_state(&mut last, &garbled(), &garbled(), 1_000, (0, 0));
		assert_eq!((said(&captured, record), said(&captured, marker)), (1, 1), "not again while they stay unreadable");
		record_state(&mut last, &Ok(Recorded::default()), &Ok(None), 1_000, (0, 0));
		record_state(&mut last, &garbled(), &garbled(), 1_000, (0, 0));
		assert_eq!((said(&captured, record), said(&captured, marker)), (2, 2), "again once each turns unreadable after a read");
	}
}

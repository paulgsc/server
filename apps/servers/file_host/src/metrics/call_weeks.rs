//! The weekly call's metrics.
//!
//! The `call_weeks` worker that keeps each week (CW2) and what home's render
//! worker has made of them (CW5), for the CALL dashboard and the `CallWeeks*`
//! and `CallRender*` alerts (`infra/prometheus/rules/home.yml`). No label
//! carries a week: counts and times only.

use crate::call_weeks::{
	pass::Pass,
	render::{Disk, Renders},
};
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> f64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

#[allow(clippy::cast_precision_loss)] // counts of weeks and Unix seconds, far below 2^52
const fn as_f64(value: i64) -> f64 {
	value as f64
}

fn count(value: usize) -> f64 {
	as_f64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// The worker started: its pass reads as not failing until one does.
pub fn record_started() {
	metrics::gauge!("call_weeks_pass_failing").set(0.0);
}

/// A pass begins.
pub fn record_pass_started() {
	metrics::gauge!("call_weeks_last_attempt_timestamp_seconds").set(now());
}

/// Whether the pass failed: storage, or any week it could not read or write.
pub fn record_pass(result: &Result<Pass, sqlx::Error>) {
	let failing = !result.as_ref().is_ok_and(|done| done.failures.is_empty());
	metrics::gauge!("call_weeks_pass_failing").set(if failing { 1.0 } else { 0.0 });
}

/// Whether the week the weekly routine owes is overdue: 1, 0, or `NaN`.
///
/// Overdue is its Friday at home passed without it reaching home
/// (`feeds::renders::overdue`, paulgsc/some-ui#3478). Set by each pass, and
/// `NaN` when the pass could not read it (`None`): unknown, never the last
/// answer (#1207).
pub fn record_next_overdue(overdue: Option<bool>) {
	metrics::gauge!("call_weeks_next_overdue").set(overdue.map_or(f64::NAN, |overdue| if overdue { 1.0 } else { 0.0 }));
}

/// What the render worker has made of the weeks home holds. A time that is
/// not there reads 0, and the worker's `last-pass` that can't be read `NaN`.
pub fn record_renders(seen: &Renders) {
	metrics::gauge!("call_weeks_held").set(count(seen.held));
	metrics::gauge!("call_render_weeks_waiting").set(count(seen.waiting));
	metrics::gauge!("call_render_weeks_let_go").set(count(seen.let_go));
	metrics::gauge!("call_render_weeks_failed").set(count(seen.failed));
	metrics::gauge!("call_render_oldest_waiting_taken_timestamp_seconds").set(seen.oldest_waiting_taken.map_or(0.0, as_f64));
	metrics::gauge!("call_render_worker_last_pass_timestamp_seconds").set(seen.worker_last_pass.gauge());
}

/// What the render worker has made of the weeks is unknown: the tick could
/// not read it. Every gauge [`record_renders`] sets is `NaN`, a gap, never
/// its last value (#1207, OA6).
pub fn record_renders_unknown() {
	for name in [
		"call_weeks_held",
		"call_render_weeks_waiting",
		"call_render_weeks_let_go",
		"call_render_weeks_failed",
		"call_render_oldest_waiting_taken_timestamp_seconds",
		"call_render_worker_last_pass_timestamp_seconds",
	] {
		metrics::gauge!(name).set(f64::NAN);
	}
}

fn bytes(value: u64) -> f64 {
	as_f64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// What the call takes of the drive that keeps it. A cache warning that is
/// not there reads 0, and one that can't be read `NaN`; a drive `statvfs`
/// could not read is `NaN`, never 0 free (#1207).
pub fn record_disk(seen: &Disk) {
	metrics::gauge!("call_render_videos_bytes").set(bytes(seen.videos_bytes));
	metrics::gauge!("call_render_work_bytes").set(bytes(seen.work_bytes));
	metrics::gauge!("call_render_cache_warning_timestamp_seconds").set(seen.cache_warning_at.gauge());
	let (free, size) = seen.drive.map_or((f64::NAN, f64::NAN), |(free, size)| (bytes(free), bytes(size)));
	metrics::gauge!("call_render_drive_avail_bytes").set(free);
	metrics::gauge!("call_render_drive_size_bytes").set(size);
}

#[cfg(test)]
mod tests {
	use super::{record_disk, record_next_overdue, record_renders, record_renders_unknown};
	use crate::call_weeks::render::{Disk, Reading, Renders};
	use metrics_util::debugging::{DebugValue, DebuggingRecorder};

	/// Every gauge `record` set, by name.
	fn gauges(record: impl FnOnce()) -> Vec<(String, f64)> {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		metrics::with_local_recorder(&recorder, record);
		snapshotter
			.snapshot()
			.into_vec()
			.into_iter()
			.filter_map(|(key, _, _, value)| match value {
				DebugValue::Gauge(value) => Some((key.key().name().to_owned(), value.into_inner())),
				_ => None,
			})
			.collect()
	}

	fn value(gauges: &[(String, f64)], name: &str) -> f64 {
		gauges.iter().find(|(n, _)| n == name).map(|(_, v)| *v).unwrap()
	}

	/// #1207 (OA6): a signal not read is `NaN`, a gap, never 0 or its last
	/// value; one that is not there (no warning, no pass yet) stays 0.
	/// Family: the renders unread, a `last-pass` and a cache warning
	/// unreadable or absent, a drive `statvfs` could not read, and whether
	/// the next week is overdue unread.
	#[test]
	fn a_signal_not_read_is_unknown_and_one_not_there_is_zero() {
		let unknown = gauges(record_renders_unknown);
		assert_eq!(unknown.len(), 6, "{unknown:?}");
		assert!(unknown.iter().all(|(_, v)| v.is_nan()), "{unknown:?}");

		let unreadable = Renders {
			worker_last_pass: Reading::Unreadable,
			..Renders::default()
		};
		let set = gauges(|| record_renders(&unreadable));
		assert!(value(&set, "call_render_worker_last_pass_timestamp_seconds").is_nan());
		assert!(value(&set, "call_render_weeks_waiting").abs() < f64::EPSILON, "the rest read");
		let set = gauges(|| record_renders(&Renders::default()));
		assert!(value(&set, "call_render_worker_last_pass_timestamp_seconds").abs() < f64::EPSILON, "no pass yet is 0");

		let disk = Disk {
			cache_warning_at: Reading::Unreadable,
			drive: None,
			..Disk::default()
		};
		let set = gauges(|| record_disk(&disk));
		for name in [
			"call_render_cache_warning_timestamp_seconds",
			"call_render_drive_avail_bytes",
			"call_render_drive_size_bytes",
		] {
			assert!(value(&set, name).is_nan(), "{name}");
		}
		let disk = Disk {
			drive: Some((1, 2)),
			..Disk::default()
		};
		let set = gauges(|| record_disk(&disk));
		assert!(value(&set, "call_render_cache_warning_timestamp_seconds").abs() < f64::EPSILON, "no warning is 0");
		assert!((value(&set, "call_render_drive_avail_bytes") - 1.0).abs() < f64::EPSILON);

		assert!(value(&gauges(|| record_next_overdue(None)), "call_weeks_next_overdue").is_nan());
		assert!((value(&gauges(|| record_next_overdue(Some(true))), "call_weeks_next_overdue") - 1.0).abs() < f64::EPSILON);
	}

	/// #1207: two jobs' newest cache warning keeps a time read, a lower
	/// bound, beside one unreadable, and is unknown only when nothing read.
	#[test]
	fn the_newest_of_two_readings_keeps_a_time_read_and_is_unknown_only_when_none_is() {
		let (absent, unreadable) = (Reading::Absent, Reading::Unreadable);
		assert_eq!(Reading::Read(1).newest(Reading::Read(2)), Reading::Read(2));
		assert_eq!(Reading::Read(1).newest(absent), Reading::Read(1));
		assert_eq!(absent.newest(absent), absent);
		assert_eq!(Reading::Read(1).newest(unreadable), Reading::Read(1));
		assert_eq!(unreadable.newest(Reading::Read(1)), Reading::Read(1));
		assert_eq!(unreadable.newest(absent), unreadable);
		assert_eq!(absent.newest(unreadable), unreadable);
	}

	/// #1207: `record_renders_unknown` empties exactly the gauges
	/// `record_renders` sets, no more and no fewer.
	#[test]
	fn renders_unknown_names_every_gauge_renders_sets() {
		let names = |set: Vec<(String, f64)>| {
			let mut names: Vec<String> = set.into_iter().map(|(name, _)| name).collect();
			names.sort();
			names
		};
		assert_eq!(names(gauges(record_renders_unknown)), names(gauges(|| record_renders(&Renders::default()))));
	}
}

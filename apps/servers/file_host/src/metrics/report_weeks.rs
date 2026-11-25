//! The engagement report's metrics (paulgsc/some-ui ENG12, #2700).
//!
//! The `report_weeks` worker keeps each ended week's report and gives it to
//! home's render worker (paulgsc/server#821); these are for the CALL
//! dashboard's engagement report row and the `ReportWeeksStale`,
//! `ReportWeeksFailing`, `ReportRenderFailed` and `ReportRenderOverdue`
//! alerts (`infra/prometheus/rules/home.yml`). No label carries a week:
//! counts and flags only.

use crate::call_weeks::render::Renders;
use crate::report_weeks::{Asked, Pass, Tidied};

#[allow(clippy::cast_precision_loss)] // counts of weeks, far below 2^52
const fn count(value: usize) -> f64 {
	value as f64
}

#[allow(clippy::cast_precision_loss)] // Unix seconds, far below 2^52
const fn seconds(value: i64) -> f64 {
	value as f64
}

/// The worker started: its pass and tidy read as not failing until one does.
pub fn record_started() {
	metrics::gauge!("report_weeks_pass_failing").set(0.0);
	metrics::gauge!("report_weeks_tidy_failing").set(0.0);
}

/// A pass started: when, for `ReportWeeksStale`.
#[allow(clippy::cast_precision_loss)] // Unix seconds, far below 2^52
pub fn record_pass_started() {
	metrics::gauge!("report_weeks_last_attempt_timestamp_seconds").set(chrono::Utc::now().timestamp() as f64);
}

/// Whether the minute's tidy failed: storage, or a file of a week no longer
/// the owner's that the drive would not remove.
pub fn record_tidy(result: &Result<Tidied, sqlx::Error>) {
	let failing = !result.as_ref().is_ok_and(|done| done.failed == 0);
	metrics::gauge!("report_weeks_tidy_failing").set(if failing { 1.0 } else { 0.0 });
}

/// Whether the pass failed (storage, or a week it could not write to the
/// drive), how many weeks home holds, and whether the last whole week had
/// no engagement record to report.
pub fn record_pass(result: &Result<Pass, sqlx::Error>) {
	let failing = !result.as_ref().is_ok_and(|done| done.failed == 0);
	metrics::gauge!("report_weeks_pass_failing").set(if failing { 1.0 } else { 0.0 });
	if let Ok(done) = result {
		metrics::gauge!("report_weeks_held").set(count(done.held));
		metrics::gauge!("report_weeks_unmeasured").set(if done.asked == Asked::Unmeasured { 1.0 } else { 0.0 });
	}
}

/// What the render worker has made of the weeks given. A time that is not
/// there reads 0, and the worker's `last-pass` that can't be read `NaN`
/// (#1207).
pub fn record_renders(seen: &Renders) {
	metrics::gauge!("report_render_weeks_waiting").set(count(seen.waiting));
	metrics::gauge!("report_render_weeks_failed").set(count(seen.failed));
	metrics::gauge!("report_render_oldest_waiting_taken_timestamp_seconds").set(seen.oldest_waiting_taken.map_or(0.0, seconds));
	metrics::gauge!("report_render_worker_last_pass_timestamp_seconds").set(seen.worker_last_pass.gauge());
}

/// What the render worker has made is unknown: the tick could not read it.
/// Every gauge [`record_renders`] sets is `NaN`, a gap, never its last
/// value (#1227, OA6).
pub fn record_renders_unknown() {
	for name in [
		"report_render_weeks_waiting",
		"report_render_weeks_failed",
		"report_render_oldest_waiting_taken_timestamp_seconds",
		"report_render_worker_last_pass_timestamp_seconds",
	] {
		metrics::gauge!(name).set(f64::NAN);
	}
}

#[cfg(test)]
mod tests {
	use super::{record_renders, record_renders_unknown, Renders};
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

	/// #1227 (OA6): renders the tick could not read are `NaN` on exactly the
	/// gauges `record_renders` sets, a gap, never their last values.
	#[test]
	fn renders_not_read_are_unknown_on_every_gauge_renders_sets() {
		let unknown = gauges(record_renders_unknown);
		assert!(unknown.iter().all(|(_, value)| value.is_nan()), "{unknown:?}");
		let names = |set: Vec<(String, f64)>| {
			let mut names: Vec<String> = set.into_iter().map(|(name, _)| name).collect();
			names.sort();
			names
		};
		assert_eq!(names(unknown), names(gauges(|| record_renders(&Renders::default()))));
	}
}

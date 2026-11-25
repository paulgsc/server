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

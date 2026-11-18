//! The engagement report's metrics (paulgsc/some-ui ENG12, #2700).
//!
//! The `report_weeks` worker keeps each ended week's report and gives it to
//! home's render worker (paulgsc/server#821); these are for the CALL
//! dashboard's engagement report row and the `ReportWeeksFailing` alert
//! (`infra/prometheus/rules/home.yml`). No label carries a week: counts and
//! flags only.

use crate::report_weeks::{Asked, Pass};

#[allow(clippy::cast_precision_loss)] // counts of weeks, far below 2^52
const fn count(value: usize) -> f64 {
	value as f64
}

/// The worker started: its pass reads as not failing until one does.
pub fn record_started() {
	metrics::gauge!("report_weeks_pass_failing").set(0.0);
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

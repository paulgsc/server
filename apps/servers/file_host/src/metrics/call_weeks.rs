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

/// What the render worker has made of the weeks home holds. A time that is
/// not there reads 0.
pub fn record_renders(seen: &Renders) {
	metrics::gauge!("call_weeks_held").set(count(seen.held));
	metrics::gauge!("call_render_weeks_waiting").set(count(seen.waiting));
	metrics::gauge!("call_render_weeks_let_go").set(count(seen.let_go));
	metrics::gauge!("call_render_weeks_failed").set(count(seen.failed));
	metrics::gauge!("call_render_oldest_waiting_taken_timestamp_seconds").set(seen.oldest_waiting_taken.map_or(0.0, as_f64));
	metrics::gauge!("call_render_worker_last_pass_timestamp_seconds").set(seen.worker_last_pass.map_or(0.0, as_f64));
}

fn bytes(value: u64) -> f64 {
	as_f64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// What the call takes of the drive that keeps it. A time or a drive that
/// is not there reads 0.
pub fn record_disk(seen: &Disk) {
	metrics::gauge!("call_render_videos_bytes").set(bytes(seen.videos_bytes));
	metrics::gauge!("call_render_work_bytes").set(bytes(seen.work_bytes));
	metrics::gauge!("call_render_cache_warning_timestamp_seconds").set(seen.cache_warning_at.map_or(0.0, as_f64));
	let (free, size) = seen.drive.unwrap_or((0, 0));
	metrics::gauge!("call_render_drive_avail_bytes").set(bytes(free));
	metrics::gauge!("call_render_drive_size_bytes").set(bytes(size));
}

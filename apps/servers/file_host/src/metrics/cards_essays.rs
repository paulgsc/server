//! The cards essays' metrics (CARDS7, #854).
//!
//! The `cards_essays` worker that keeps each essay and what home's render
//! worker has made of them, for the `CardsEssay*` alerts
//! (`infra/prometheus/rules/home.yml`). No label carries an essay: counts and
//! times only.

use crate::call_weeks::render::Renders;
use crate::cards_essays::pass::Pass;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> f64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

#[allow(clippy::cast_precision_loss)] // counts of essays and Unix seconds, far below 2^52
const fn as_f64(value: i64) -> f64 {
	value as f64
}

fn count(value: usize) -> f64 {
	as_f64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// The worker started: its pass reads as not failing until one does.
pub fn record_started() {
	metrics::gauge!("cards_essays_pass_failing").set(0.0);
}

/// A pass begins.
pub fn record_pass_started() {
	metrics::gauge!("cards_essays_last_attempt_timestamp_seconds").set(now());
}

/// Whether the pass failed: storage, or any essay it could not read or write.
pub fn record_pass(result: &Result<Pass, sqlx::Error>) {
	let failing = !result.as_ref().is_ok_and(|done| done.failures.is_empty());
	metrics::gauge!("cards_essays_pass_failing").set(if failing { 1.0 } else { 0.0 });
}

/// What the render worker has made of the essays home holds. A time that is
/// not there reads 0.
pub fn record_renders(seen: &Renders) {
	metrics::gauge!("cards_essay_render_waiting").set(count(seen.waiting));
	metrics::gauge!("cards_essay_render_failed").set(count(seen.failed));
	metrics::gauge!("cards_essay_render_oldest_waiting_taken_timestamp_seconds").set(seen.oldest_waiting_taken.map_or(0.0, as_f64));
}

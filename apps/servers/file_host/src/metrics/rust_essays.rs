//! Rust Rounds essays' metrics (RR16, paulgsc/some-ui#2691).
//!
//! The `rust_essays` worker that gives each essay to home's render worker,
//! and what the render worker has made of them, for the CALL dashboard and
//! the `RustEssay*` alerts (`infra/prometheus/rules/home.yml`). No label
//! carries a week: counts and times only.

use crate::rust_essays::{Exported, Seen};

#[allow(clippy::cast_precision_loss)] // counts of essays and Unix seconds, far below 2^52
const fn as_f64(value: i64) -> f64 {
	value as f64
}

fn count(value: usize) -> f64 {
	as_f64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// The worker started: its pass reads as not failing until one does.
pub fn record_started() {
	metrics::gauge!("rust_essays_pass_failing").set(0.0);
}

/// Whether the pass failed: storage, or an essay it could not write.
pub fn record_pass(result: &Result<Exported, sqlx::Error>) {
	let failing = !result.as_ref().is_ok_and(|done| done.failed == 0);
	metrics::gauge!("rust_essays_pass_failing").set(if failing { 1.0 } else { 0.0 });
	if let Ok(done) = result {
		metrics::gauge!("rust_essays_waiting_for_deadline").set(count(done.waiting));
	}
}

/// What the render worker has made of the essays given. A time that is not
/// there reads 0.
pub fn record_renders(seen: &Seen) {
	metrics::gauge!("rust_essays_refused").set(count(seen.refused));
	metrics::gauge!("rust_essay_render_given").set(count(seen.renders.held));
	metrics::gauge!("rust_essay_render_waiting").set(count(seen.renders.waiting));
	metrics::gauge!("rust_essay_render_failed").set(count(seen.renders.failed));
	metrics::gauge!("rust_essay_render_oldest_waiting_taken_timestamp_seconds").set(seen.renders.oldest_waiting_taken.map_or(0.0, as_f64));
	metrics::gauge!("rust_essay_render_worker_last_pass_timestamp_seconds").set(seen.renders.worker_last_pass.map_or(0.0, as_f64));
}

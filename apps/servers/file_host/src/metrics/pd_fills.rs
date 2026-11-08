//! Playoff Delta's fills' metrics.
//!
//! The `pd_fills` worker that keeps each fill (PK2), for the
//! `PlayoffDeltaFills*` alerts (`infra/prometheus/rules/home.yml`). No label
//! carries an episode: times and one flag only.

use crate::pd_fills::pass::Pass;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> f64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// The worker started: its pass reads as not failing until one does.
pub fn record_started() {
	metrics::gauge!("pd_fills_pass_failing").set(0.0);
}

/// A pass begins.
pub fn record_pass_started() {
	metrics::gauge!("pd_fills_last_attempt_timestamp_seconds").set(now());
}

/// Whether the pass failed: storage, or any fill it could not read.
pub fn record_pass(result: &Result<Pass, sqlx::Error>) {
	let failing = !result.as_ref().is_ok_and(|done| done.failures.is_empty());
	metrics::gauge!("pd_fills_pass_failing").set(if failing { 1.0 } else { 0.0 });
}

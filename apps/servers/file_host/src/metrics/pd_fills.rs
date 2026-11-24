//! Playoff Delta's fills' metrics.
//!
//! The `pd_fills` worker that keeps each fill (PK2) and what home's render
//! worker has made of them (PK5), for the CALL dashboard and the
//! `PlayoffDelta*` alerts (`infra/prometheus/rules/home.yml`). No label
//! carries an episode: counts and times only.

use crate::pd_fills::{pass::Pass, render::Seen};
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

#[allow(clippy::cast_precision_loss)] // counts of fills and Unix seconds, far below 2^52
const fn as_f64(value: i64) -> f64 {
	value as f64
}

fn count(value: usize) -> f64 {
	as_f64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// What the render worker has made of the fills home holds. A time that is
/// not there reads 0.
pub fn record_renders(seen: &Seen) {
	metrics::gauge!("pd_fills_held").set(count(seen.fills));
	metrics::gauge!("pd_render_episodes_unvoiced").set(count(seen.unvoiced));
	metrics::gauge!("pd_render_episodes_waiting").set(count(seen.renders.waiting));
	metrics::gauge!("pd_render_episodes_failed").set(count(seen.renders.failed));
	metrics::gauge!("pd_render_episodes_let_go").set(count(seen.renders.let_go));
	metrics::gauge!("pd_render_oldest_waiting_taken_timestamp_seconds").set(seen.renders.oldest_waiting_taken.map_or(0.0, as_f64));
}

/// What the render worker has made of the fills is unknown: the tick could
/// not read it. Every gauge [`record_renders`] sets is `NaN`, a gap, never
/// its last value (#1207, OA6).
pub fn record_renders_unknown() {
	for name in [
		"pd_fills_held",
		"pd_render_episodes_unvoiced",
		"pd_render_episodes_waiting",
		"pd_render_episodes_failed",
		"pd_render_episodes_let_go",
		"pd_render_oldest_waiting_taken_timestamp_seconds",
	] {
		metrics::gauge!(name).set(f64::NAN);
	}
}

#[cfg(test)]
mod tests {
	use metrics_util::debugging::{DebugValue, DebuggingRecorder};

	/// #1207 (OA6): fills the tick could not read are `NaN` on every gauge
	/// `record_renders` sets, a gap, never their last values.
	#[test]
	fn renders_not_read_are_unknown() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		metrics::with_local_recorder(&recorder, super::record_renders_unknown);
		let set: Vec<_> = snapshotter
			.snapshot()
			.into_vec()
			.into_iter()
			.filter_map(|(key, _, _, value)| match value {
				DebugValue::Gauge(value) => Some((key.key().name().to_owned(), value.into_inner())),
				_ => None,
			})
			.collect();
		assert_eq!(set.len(), 6, "{set:?}");
		assert!(set.iter().all(|(_, value)| value.is_nan()), "{set:?}");
	}
}

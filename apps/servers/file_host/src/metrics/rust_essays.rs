//! Essays' metrics, each kind under its own names: Rust Rounds' (RR16,
//! paulgsc/some-ui#2691) as `rust_essay*`, a week of mock interview's (IW6,
//! paulgsc/some-ui#2937) as `interview_essay*`.
//!
//! The `rust_essays` worker that gives each essay to home's render worker, and
//! what the render worker has made of them, for the CALL dashboard and the
//! `RustEssay*` and `InterviewEssay*` alerts
//! (`infra/prometheus/rules/home.yml`). No label carries a week: counts and
//! times only.

use crate::rust_essays::{Exported, Kind, Seen};

/// A kind's gauges' names.
struct Names {
	pass_failing: &'static str,
	waiting_for_deadline: &'static str,
	refused: &'static str,
	given: &'static str,
	waiting: &'static str,
	failed: &'static str,
	oldest_waiting: &'static str,
	worker_last_pass: &'static str,
}

const fn names(kind: Kind) -> &'static Names {
	match kind {
		Kind::RustRounds => &Names {
			pass_failing: "rust_essays_pass_failing",
			waiting_for_deadline: "rust_essays_waiting_for_deadline",
			refused: "rust_essays_refused",
			given: "rust_essay_render_given",
			waiting: "rust_essay_render_waiting",
			failed: "rust_essay_render_failed",
			oldest_waiting: "rust_essay_render_oldest_waiting_taken_timestamp_seconds",
			worker_last_pass: "rust_essay_render_worker_last_pass_timestamp_seconds",
		},
		Kind::Interview => &Names {
			pass_failing: "interview_essays_pass_failing",
			waiting_for_deadline: "interview_essays_waiting_for_deadline",
			refused: "interview_essays_refused",
			given: "interview_essay_render_given",
			waiting: "interview_essay_render_waiting",
			failed: "interview_essay_render_failed",
			oldest_waiting: "interview_essay_render_oldest_waiting_taken_timestamp_seconds",
			worker_last_pass: "interview_essay_render_worker_last_pass_timestamp_seconds",
		},
	}
}

#[allow(clippy::cast_precision_loss)] // counts of essays and Unix seconds, far below 2^52
const fn as_f64(value: i64) -> f64 {
	value as f64
}

fn count(value: usize) -> f64 {
	as_f64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// The worker started: its pass reads as not failing until one does.
pub fn record_started(kind: Kind) {
	metrics::gauge!(names(kind).pass_failing).set(0.0);
}

/// Whether the pass failed: storage, or an essay it could not write.
pub fn record_pass(kind: Kind, result: &Result<Exported, sqlx::Error>) {
	let names = names(kind);
	let failing = !result.as_ref().is_ok_and(|done| done.failed == 0);
	metrics::gauge!(names.pass_failing).set(if failing { 1.0 } else { 0.0 });
	if let Ok(done) = result {
		metrics::gauge!(names.waiting_for_deadline).set(count(done.waiting));
	}
}

/// What the render worker has made of the essays given. A time that is not
/// there reads 0.
pub fn record_renders(kind: Kind, seen: &Seen) {
	let names = names(kind);
	metrics::gauge!(names.refused).set(count(seen.refused));
	metrics::gauge!(names.given).set(count(seen.renders.held));
	metrics::gauge!(names.waiting).set(count(seen.renders.waiting));
	metrics::gauge!(names.failed).set(count(seen.renders.failed));
	metrics::gauge!(names.oldest_waiting).set(seen.renders.oldest_waiting_taken.map_or(0.0, as_f64));
	metrics::gauge!(names.worker_last_pass).set(seen.renders.worker_last_pass.map_or(0.0, as_f64));
}

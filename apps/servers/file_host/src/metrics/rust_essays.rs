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

// Each kind's gauges, by name: written out whole, so the metric contract
// check (`scripts/check_metric_contract.py`) sees every name emitted.
const RUST_PASS_FAILING: &str = "rust_essays_pass_failing";
const INTERVIEW_PASS_FAILING: &str = "interview_essays_pass_failing";
const RUST_WAITING_FOR_DEADLINE: &str = "rust_essays_waiting_for_deadline";
const INTERVIEW_WAITING_FOR_DEADLINE: &str = "interview_essays_waiting_for_deadline";
const RUST_REFUSED: &str = "rust_essays_refused";
const INTERVIEW_REFUSED: &str = "interview_essays_refused";
const RUST_GIVEN: &str = "rust_essay_render_given";
const INTERVIEW_GIVEN: &str = "interview_essay_render_given";
const RUST_WAITING: &str = "rust_essay_render_waiting";
const INTERVIEW_WAITING: &str = "interview_essay_render_waiting";
const RUST_FAILED: &str = "rust_essay_render_failed";
const INTERVIEW_FAILED: &str = "interview_essay_render_failed";
const RUST_OLDEST_WAITING: &str = "rust_essay_render_oldest_waiting_taken_timestamp_seconds";
const INTERVIEW_OLDEST_WAITING: &str = "interview_essay_render_oldest_waiting_taken_timestamp_seconds";
const RUST_OWED_OVERDUE: &str = "rust_essays_owed_overdue";
const INTERVIEW_OWED_OVERDUE: &str = "interview_essays_owed_overdue";
const RUST_WORKER_LAST_PASS: &str = "rust_essay_render_worker_last_pass_timestamp_seconds";
const INTERVIEW_WORKER_LAST_PASS: &str = "interview_essay_render_worker_last_pass_timestamp_seconds";

/// One of a kind's gauges.
#[derive(Clone, Copy)]
enum Gauge {
	PassFailing,
	WaitingForDeadline,
	Refused,
	Given,
	Waiting,
	Failed,
	OldestWaiting,
	WorkerLastPass,
	OwedOverdue,
}

/// `kind`'s `gauge`.
fn gauge(kind: Kind, gauge: Gauge) -> metrics::Gauge {
	match (kind, gauge) {
		(Kind::RustRounds, Gauge::PassFailing) => metrics::gauge!(RUST_PASS_FAILING),
		(Kind::Interview, Gauge::PassFailing) => metrics::gauge!(INTERVIEW_PASS_FAILING),
		(Kind::RustRounds, Gauge::WaitingForDeadline) => metrics::gauge!(RUST_WAITING_FOR_DEADLINE),
		(Kind::Interview, Gauge::WaitingForDeadline) => metrics::gauge!(INTERVIEW_WAITING_FOR_DEADLINE),
		(Kind::RustRounds, Gauge::Refused) => metrics::gauge!(RUST_REFUSED),
		(Kind::Interview, Gauge::Refused) => metrics::gauge!(INTERVIEW_REFUSED),
		(Kind::RustRounds, Gauge::Given) => metrics::gauge!(RUST_GIVEN),
		(Kind::Interview, Gauge::Given) => metrics::gauge!(INTERVIEW_GIVEN),
		(Kind::RustRounds, Gauge::Waiting) => metrics::gauge!(RUST_WAITING),
		(Kind::Interview, Gauge::Waiting) => metrics::gauge!(INTERVIEW_WAITING),
		(Kind::RustRounds, Gauge::Failed) => metrics::gauge!(RUST_FAILED),
		(Kind::Interview, Gauge::Failed) => metrics::gauge!(INTERVIEW_FAILED),
		(Kind::RustRounds, Gauge::OldestWaiting) => metrics::gauge!(RUST_OLDEST_WAITING),
		(Kind::Interview, Gauge::OldestWaiting) => metrics::gauge!(INTERVIEW_OLDEST_WAITING),
		(Kind::RustRounds, Gauge::WorkerLastPass) => metrics::gauge!(RUST_WORKER_LAST_PASS),
		(Kind::Interview, Gauge::WorkerLastPass) => metrics::gauge!(INTERVIEW_WORKER_LAST_PASS),
		(Kind::RustRounds, Gauge::OwedOverdue) => metrics::gauge!(RUST_OWED_OVERDUE),
		(Kind::Interview, Gauge::OwedOverdue) => metrics::gauge!(INTERVIEW_OWED_OVERDUE),
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
	gauge(kind, Gauge::PassFailing).set(0.0);
}

/// Whether the pass failed: storage, or an essay it could not write.
pub fn record_pass(kind: Kind, result: &Result<Exported, sqlx::Error>) {
	let failing = !result.as_ref().is_ok_and(|done| done.failed == 0);
	gauge(kind, Gauge::PassFailing).set(if failing { 1.0 } else { 0.0 });
	if let Ok(done) = result {
		gauge(kind, Gauge::WaitingForDeadline).set(count(done.waiting));
	}
}

/// What the render worker has made of the essays given. A time that is not
/// there reads 0, and the worker's `last-pass` that can't be read `NaN`
/// (#1207).
pub fn record_renders(kind: Kind, seen: &Seen) {
	gauge(kind, Gauge::Refused).set(count(seen.refused));
	gauge(kind, Gauge::Given).set(count(seen.renders.held));
	gauge(kind, Gauge::Waiting).set(count(seen.renders.waiting));
	gauge(kind, Gauge::Failed).set(count(seen.renders.failed));
	gauge(kind, Gauge::OldestWaiting).set(seen.renders.oldest_waiting_taken.map_or(0.0, as_f64));
	gauge(kind, Gauge::WorkerLastPass).set(seen.renders.worker_last_pass.gauge());
}

/// Whether an essay home is owed has passed its Monday unrendered.
///
/// Not given to the worker by its Monday at home (`feeds::renders::overdue`,
/// paulgsc/some-ui#3478): 1 or 0, set by each pass. The render gauges above count only essays
/// given, so a week whose essay never came read as quiet.
pub fn record_overdue(kind: Kind, overdue: bool) {
	gauge(kind, Gauge::OwedOverdue).set(if overdue { 1.0 } else { 0.0 });
}

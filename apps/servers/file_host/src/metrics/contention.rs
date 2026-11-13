//! The contention guard's counts (L1, paulgsc/server#706; `crate::contention`).
//!
//! One counter, `contention_outcomes_total`, labelled by the worker
//! (`rust_rounds`, `transcribe`) and what the guard did (`excused`,
//! `judged_anyway`, `marked`, `unreadable`): both closed sets, never a
//! program, a note or a subject. Read by the CALL dashboard's LANE row.

/// What one pass's guard did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
	pub excused: usize,
	pub judged_anyway: usize,
	pub marked: usize,
	pub unreadable: usize,
}

/// Add one pass's counts to `worker`'s. Called with none when the worker
/// starts, so a guard that never acted reads 0 rather than absent.
pub fn record(worker: &'static str, counts: Counts) {
	for (what, count) in [
		("excused", counts.excused),
		("judged_anyway", counts.judged_anyway),
		("marked", counts.marked),
		("unreadable", counts.unreadable),
	] {
		metrics::counter!("contention_outcomes_total", "worker" => worker, "what" => what).increment(u64::try_from(count).unwrap_or(u64::MAX));
	}
}

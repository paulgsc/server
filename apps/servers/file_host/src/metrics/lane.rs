//! The batch lane's metrics (L4b, paulgsc/server#708).
//!
//! What the lane decided and why, every signal it decided from, whether each
//! could be read, and when it last wrote a decision, for the CALL
//! dashboard's LANE row, the `Lane*` alerts in
//! `infra/prometheus/rules/home.yml`, and WHO DUNNIT's idle footprint
//! (`lane_idle`). No label carries a job id or a week: the verb, the reason
//! and the signal's name are the only labels, each from a closed set.

use crate::call_weeks::lane::Readable;
use ::lane::{Decision, Policy, Signals, Verb, Why};

const VERBS: [Verb; 3] = [Verb::Run, Verb::Pause, Verb::Stop];
const WHYS: [Why; 6] = [Why::Clear, Why::Busy, Why::Memory, Why::Pressure, Why::Unknown, Why::Settling];

#[allow(clippy::cast_precision_loss)] // bytes and Unix seconds, far below 2^52
const fn as_f64(value: u64) -> f64 {
	value as f64
}

fn signed(value: i64) -> f64 {
	as_f64(u64::try_from(value).unwrap_or(0))
}

const fn flag(on: bool) -> f64 {
	if on {
		1.0
	} else {
		0.0
	}
}

/// The lane started: its policy, and no decision written yet, so a lane
/// that never writes one reads as stale (`LaneStale`).
pub fn record_started(policy: &Policy, declared_peak_bytes: u64) {
	metrics::gauge!("lane_floor_bytes").set(as_f64(policy.floor_bytes));
	metrics::gauge!("lane_pressure_limit_pct").set(policy.pressure_limit_pct);
	metrics::gauge!("lane_declared_peak_bytes").set(as_f64(declared_peak_bytes));
	metrics::gauge!("lane_last_decision_timestamp_seconds").set(0.0);
}

/// One tick: the decision, the signals, and whether it was written. A
/// signal that could not be read is `NaN`, a gap on the graph, and says so
/// on `lane_signal_readable`.
pub fn record(decision: &Decision, signals: &Signals, readable: &Readable, idle: bool, batch_active: bool, written_at: Option<i64>) {
	for verb in VERBS {
		metrics::gauge!("lane_verb", "verb" => verb.name()).set(flag(verb == decision.verb));
	}
	for why in WHYS {
		metrics::gauge!("lane_why", "why" => why.name()).set(flag(why == decision.why));
	}
	metrics::counter!("lane_ticks_total", "verb" => decision.verb.name(), "why" => decision.why.name()).increment(1);
	metrics::gauge!("lane_stops").set(f64::from(decision.stops));
	metrics::gauge!("lane_stuck").set(flag(decision.stuck));
	metrics::gauge!("lane_mem_available_bytes").set(signals.mem_available_bytes.map_or(f64::NAN, as_f64));
	metrics::gauge!("lane_memory_pressure_pct").set(signals.memory_pressure_pct.unwrap_or(f64::NAN));
	metrics::gauge!("lane_batch_peak_bytes").set(signals.batch_peak_bytes.map_or(f64::NAN, as_f64));
	metrics::gauge!("lane_interactive_waiting").set(flag(signals.interactive_waiting));
	metrics::gauge!("lane_batch_job_present").set(flag(signals.batch_job.is_some()));
	metrics::gauge!("lane_batch_active").set(flag(batch_active));
	metrics::gauge!("lane_idle").set(flag(idle));
	for (signal, ok) in [
		("meminfo", readable.meminfo),
		("pressure", readable.pressure),
		("job", readable.job),
		("peak", readable.peak),
	] {
		metrics::gauge!("lane_signal_readable", "signal" => signal).set(flag(ok));
	}
	if let Some(at) = written_at {
		metrics::gauge!("lane_last_decision_timestamp_seconds").set(signed(at));
	}
}

#[cfg(test)]
mod tests {
	use super::{VERBS, WHYS};
	use ::lane::{Verb, Why};

	/// Every verb and reason has its series: one added to `crates/lane`
	/// fails here until it is listed.
	#[test]
	fn every_verb_and_reason_is_listed() {
		for verb in VERBS {
			match verb {
				Verb::Run | Verb::Pause | Verb::Stop => {}
			}
		}
		for why in WHYS {
			match why {
				Why::Clear | Why::Busy | Why::Memory | Why::Pressure | Why::Unknown | Why::Settling => {}
			}
		}
		let distinct: std::collections::BTreeSet<_> = WHYS.iter().map(|why| why.name()).collect();
		assert_eq!(distinct.len(), WHYS.len(), "each listed once");
	}
}

//! `refusals_total{stage,reason}` — see #213 (F3).
//!
//! This server declines work in several distinct ways, and refusal is fault
//! condition #4 — **saturated** — the one that looks healthiest from every
//! other angle: every request that *is* served stays fast, because the slow
//! ones were never admitted. Before this, the only evidence was a `warn!`
//! or `error!` line nobody was watching. These counters sit next to those
//! log lines, not in place of them — the log says what happened, the
//! metric says when to look.
//!
//! `stage` is `"http"` or `"ws"`. `reason` is one of:
//!
//!   http: `timeout`, `load_shed`, `body_limit`
//!   ws:   `global_capacity`, `queue_full`, `permit_timeout`
//!
//! One row from #213's refusal table is deliberately not counted here:
//!
//! - **Concurrency limit** (`tower::limit::ConcurrencyLimitLayer`) — in the
//!   tower 0.4 this workspace pins, `ConcurrencyLimit::poll_ready` never
//!   produces an error; it is pure backpressure (`Poll::Pending` on a
//!   semaphore) with no discrete "refused" event to count. When sustained
//!   concurrency pressure does turn into an actual refusal, that refusal
//!   happens at a layer this module *does* count — `load_shed` if
//!   `LoadShedLayer` sheds it, `timeout` if `TimeoutLayer` elapses first.
//!   Inventing a synthetic "queued" counter here would mean fabricating a
//!   signal tower itself doesn't produce, which is exactly what this story
//!   (and #223's "observe first" non-goal) argues against.

use metrics::counter;

/// Every `(stage, reason)` pair this module can record — the table above.
pub const REASONS: [(&str, &str); 6] = [
	("http", "timeout"),
	("http", "load_shed"),
	("http", "body_limit"),
	("ws", "global_capacity"),
	("ws", "queue_full"),
	("ws", "permit_timeout"),
];

/// Start every refusal series at 0, once at boot.
///
/// A counter the `metrics` facade has never incremented doesn't exist, and
/// "never refused anything" must not read as "not measured": the HEALTH row's
/// REFUSALS reads `refusals_total`, and SIGNAL calls a missing one blind. Until
/// the blackbox WS probe stopped refusing itself (infra/blackbox.yml), every
/// process got its first `permit_timeout` within seconds, which hid this.
pub fn register_all() {
	for (stage, reason) in REASONS {
		counter!("refusals_total", "stage" => stage, "reason" => reason).increment(0);
	}
}

pub fn record_http(reason: &'static str) {
	counter!("refusals_total", "stage" => "http", "reason" => reason).increment(1);
}

pub fn record_ws(reason: &'static str) {
	counter!("refusals_total", "stage" => "ws", "reason" => reason).increment(1);
}

#[cfg(test)]
mod tests {
	use metrics_util::debugging::{DebugValue, DebuggingRecorder};

	/// A freshly booted server with no refusals still exports every series,
	/// at zero — not an absent family.
	#[test]
	fn every_refusal_series_exists_at_zero_after_register_all() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		metrics::with_local_recorder(&recorder, super::register_all);

		let snapshot = snapshotter.snapshot().into_vec();
		for (stage, reason) in super::REASONS {
			let value = snapshot.iter().find_map(|(key, _, _, value)| {
				let key = key.key();
				let is_it =
					key.name() == "refusals_total" && key.labels().any(|l| l.key() == "stage" && l.value() == stage) && key.labels().any(|l| l.key() == "reason" && l.value() == reason);
				is_it.then_some(value)
			});
			assert!(matches!(value, Some(DebugValue::Counter(0))), "{stage}/{reason}: {value:?}");
		}
	}
}

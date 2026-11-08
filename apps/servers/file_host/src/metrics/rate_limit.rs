//! `rate_limit_decisions_total{limiter,outcome}` and the limiter's own state
//! gauges — see #215 (A1/A2).
//!
//! `main.rs:84` used to carry a bare `TODO: Is this even working! boyo needs
//! to know!` next to the limiter's installation. It is now falsifiable:
//! `sum(rate(rate_limit_decisions_total{outcome="rejected"}[5m]))` answers
//! whether the limiter has ever rejected anything, on any deployment, which
//! was previously not recoverable from any artifact the server produces
//! (#211).
//!
//! `limiter` is fixed at `"http"` today — the only limiter this server has —
//! but the label exists from the start because the push path already
//! receives its own `429`s from browser push services
//! (`push_kit::SendOutcome::RateLimited`) and a future story can reuse this
//! same shape rather than inventing a second metric name.
//!
//! `client_id` never appears as a label on any of these: it is the
//! partition key `some_services::rate_limiter::KeyedLimiter`
//! uses internally, and promoting it to a metric label would be exactly the
//! unbounded-cardinality mistake #222 (F2) already declined for
//! `http_requests_total`.

use metrics::{counter, gauge};

/// The outcomes `record_decision` is called with.
pub const OUTCOMES: [&str; 2] = ["allowed", "rejected"];

pub fn record_decision(outcome: &'static str) {
	counter!("rate_limit_decisions_total", "limiter" => "http", "outcome" => outcome).increment(1);
}

/// Start every outcome at 0, once at boot.
///
/// A counter the facade has never incremented does not exist, so the first
/// refusal after a restart would appear as a new series already at 1, which
/// `increase()` cannot see. `ClientRateLimited` (`infra/prometheus/rules/home.yml`)
/// reads exactly that increase, so without this the first refusal after every
/// restart would go untold.
pub fn register_all() {
	for outcome in OUTCOMES {
		counter!("rate_limit_decisions_total", "limiter" => "http", "outcome" => outcome).increment(0);
	}
}

/// Sampled off the request path.
///
/// See `metrics::periodic`, the same convention #227/#228 established for
/// gauges that need to await or scan shared state rather than update
/// synchronously at their own mutation site.
pub fn record_state(tokens_available: f64, active_clients: usize) {
	gauge!("rate_limit_tokens_available").set(tokens_available);
	#[allow(clippy::cast_precision_loss)]
	gauge!("rate_limit_active_clients").set(active_clients as f64);
}

/// The configured capacity, stamped once at startup.
///
/// Same convention as `websocket::connection::instrument::set_guard_capacity`:
/// fixed for the process's life, so no reason to recompute it every tick.
/// #232 (A3)'s invariant panel divides this by the fixed 60s window to get a
/// requests-per-second threshold that tracks `config.rate_limit` instead of
/// a hardcoded number that goes stale the first time someone retunes it.
pub fn record_capacity(max_tokens_per_minute: u32) {
	gauge!("rate_limit_capacity_per_minute").set(f64::from(max_tokens_per_minute));
}

#[cfg(test)]
mod tests {
	use metrics_util::debugging::{DebugValue, DebuggingRecorder};

	/// A freshly booted server that has refused nothing still exports the
	/// refusal series at 0, so the first refusal is an increase.
	#[test]
	fn every_decision_series_exists_at_zero_after_register_all() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		metrics::with_local_recorder(&recorder, super::register_all);

		let snapshot = snapshotter.snapshot().into_vec();
		for outcome in super::OUTCOMES {
			let value = snapshot.iter().find_map(|(key, _, _, value)| {
				let key = key.key();
				let is_it = key.name() == "rate_limit_decisions_total" && key.labels().any(|l| l.key() == "outcome" && l.value() == outcome);
				is_it.then_some(value)
			});
			assert!(matches!(value, Some(DebugValue::Counter(0))), "{outcome}: {value:?}");
		}
	}
}

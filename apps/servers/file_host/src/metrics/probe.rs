//! What home's probes found (paulgsc/some-ui#2383, CK4;
//! `crate::transcribe::probe`).
//!
//! A probe runs daily, so `probe_last_ended_timestamp_seconds` older than a
//! day and a bit means the probe stopped running, and `probe_last_passed`
//! says how the last run ended. Labels are a probe's name and a verdict only,
//! never what a probe heard.

use chrono::{DateTime, Utc};

/// One run ended with `verdict` (`pass` or `fail`) at `at`.
pub fn record_ended(probe: &'static str, verdict: &'static str, at: DateTime<Utc>) {
	metrics::counter!("probe_runs_total", "probe" => probe, "verdict" => verdict).increment(1);
	metrics::gauge!("probe_last_passed", "probe" => probe).set(if verdict == "pass" { 1.0 } else { 0.0 });
	#[allow(clippy::cast_precision_loss, reason = "seconds since 1970 fit an f64's mantissa")]
	let seconds = at.timestamp_millis() as f64 / 1000.0;
	metrics::gauge!("probe_last_ended_timestamp_seconds", "probe" => probe).set(seconds);
}

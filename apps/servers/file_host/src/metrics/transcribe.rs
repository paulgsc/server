//! Liveness and work metrics for the voice-note transcriber
//! (`crate::transcribe`).
//!
//! Shaped like `metrics::mailbox`: a pass that finds nothing to hear is the
//! healthy case, so `transcribe_last_attempt_timestamp_seconds` moves when a
//! pass starts, and stale means the task died or a note hangs past its
//! deadline. Labels are attempt kinds only, never a note or a subject.

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use voice_repo::transcript::Outcome;

/// Whether this deployment runs the transcriber: 1 or 0, set once at boot,
/// so "switched off" and "task missing" read apart.
pub fn record_enabled(enabled: bool) {
	metrics::gauge!("transcribe_enabled").set(if enabled { 1.0 } else { 0.0 });
}

/// Start every attempt kind at 0, so a kind that never happened reads 0
/// rather than absent, and the pass as not failing.
pub fn record_started() {
	for kind in Outcome::KINDS {
		metrics::counter!("transcribe_attempts_total", "outcome" => kind).increment(0);
	}
	metrics::gauge!("transcribe_pass_failing").set(0.0);
}

/// Mark the start of a pass.
pub fn record_pass_started() {
	let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
	metrics::gauge!("transcribe_last_attempt_timestamp_seconds").set(timestamp);
}

/// Whether the pass ended on a storage failure. Stays set until a pass
/// succeeds.
pub fn record_pass(succeeded: bool) {
	metrics::gauge!("transcribe_pass_failing").set(if succeeded { 0.0 } else { 1.0 });
}

/// One kept attempt: its kind, and how long it took.
pub fn record_attempt(kind: &'static str, took: Duration) {
	metrics::counter!("transcribe_attempts_total", "outcome" => kind).increment(1);
	metrics::histogram!("transcribe_attempt_seconds", "outcome" => kind).record(took.as_secs_f64());
}

//! Liveness metrics for the engagement waker (#216/P4, #236).
//!
//! A successful pass can legitimately find no due work and emit no domain
//! events.  These gauges make that quiet success distinguishable from a task
//! which stopped running.  The interval is exported too: the dashboard must
//! compare the pass age with the deployment's actual configuration rather
//! than with the default compiled into `Config`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Publish the configured interval as soon as the task is spawned.
pub fn record_interval(interval: Duration) {
	metrics::gauge!("nudge_waker_interval_seconds").set(interval.as_secs_f64());
}

/// Mark a completed, successful pass, including an empty one.
pub fn record_successful_pass() {
	let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
	metrics::gauge!("nudge_waker_last_pass_timestamp_seconds").set(timestamp);
}

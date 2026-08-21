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

/// #262 (SLI1): every row `SessionRepository::list()` hands back, summed
/// across every due subject one pass considers.
///
/// `list()` has no `WHERE`/`LIMIT` — see its own docstring — so this grows
/// with `BATCH × |sessions|` on an accumulated table rather than staying flat
/// as the fleet grows. This counter is the measurement half of that story:
/// it makes the cost visible on a dashboard the same way the characterisation
/// test in `nudge::waker`'s test module makes it visible in CI. Fixing the
/// read pattern is #263 (SLI2); until then this counter is expected to climb
/// with usage, and #263's job is to make it stop.
pub fn record_session_rows_read(count: usize) {
	#[allow(clippy::cast_possible_truncation)] // a session table is never near u64::MAX rows
	metrics::counter!("nudge_waker_session_rows_read_total").increment(count as u64);
}

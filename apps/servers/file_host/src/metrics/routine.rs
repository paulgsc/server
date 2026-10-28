//! When each scheduled routine is due and when its newest run started
//! (`crate::routine_watch`). Labelled by routine name only.

use chrono::{DateTime, Utc};

#[allow(clippy::cast_precision_loss)]
const fn seconds(at: DateTime<Utc>) -> f64 {
	at.timestamp() as f64
}

/// The latest scheduled start at or before now.
pub fn record_due(routine: &'static str, due: DateTime<Utc>) {
	metrics::gauge!("routine_due_timestamp_seconds", "routine" => routine).set(seconds(due));
}

/// The newest run's start, or 0 when home holds none.
pub fn record_last_run(routine: &'static str, started: Option<DateTime<Utc>>) {
	metrics::gauge!("routine_last_run_timestamp_seconds", "routine" => routine).set(started.map_or(0.0, seconds));
}

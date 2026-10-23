//! The mailbox's D1 usage today, as `crate::d1_usage` last read it from
//! Cloudflare, for the MAILBOX dashboard's usage row and its alerts.
//!
//! A gauge keeps its last value when a read fails, so a failure must be
//! visible on its own: `d1_usage_failing{error}` says the last read failed
//! and why, and `d1_usage_last_success_timestamp_seconds` says how old the
//! numbers are. The dashboard greys them out past three intervals.

use mailbox_kit::usage::Usage;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Every value `d1_usage_failing`'s `error` label can take:
/// `mailbox_kit::usage::UsageError::kind`.
pub const READ_ERROR_CLASSES: [&str; 5] = ["unauthorized", "api", "http", "transport", "decode"];

fn now_seconds() -> f64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// Whether this deployment reads the usage at all: 1 or 0, set once at boot.
pub fn record_enabled(enabled: bool) {
	metrics::gauge!("d1_usage_enabled").set(if enabled { 1.0 } else { 0.0 });
}

/// Publish the configured interval when the collector starts, and every
/// failure class at 0.
pub fn record_interval(interval: Duration) {
	metrics::gauge!("d1_usage_interval_seconds").set(interval.as_secs_f64());
	set_failing(None);
}

/// Mark the start of a read.
pub fn record_attempt() {
	metrics::gauge!("d1_usage_last_attempt_timestamp_seconds").set(now_seconds());
}

/// Today's numbers, from a read that worked.
#[allow(clippy::cast_precision_loss)]
pub fn record_usage(usage: &Usage) {
	metrics::gauge!("d1_usage_rows_read_today").set(usage.rows_read as f64);
	metrics::gauge!("d1_usage_rows_written_today").set(usage.rows_written as f64);
	metrics::gauge!("d1_usage_queries_today", "kind" => "read").set(usage.read_queries as f64);
	metrics::gauge!("d1_usage_queries_today", "kind" => "write").set(usage.write_queries as f64);
	if let Some(bytes) = usage.database_bytes {
		metrics::gauge!("d1_usage_database_bytes").set(bytes as f64);
	}
	metrics::gauge!("d1_usage_query_batch_p90_milliseconds").set(usage.query_batch_p90_ms.unwrap_or(0.0));
	metrics::gauge!("d1_usage_last_success_timestamp_seconds").set(now_seconds());
	set_failing(None);
}

/// A read that failed, by class. Stays set until a read works.
pub fn record_failure(class: &'static str) {
	set_failing(Some(class));
}

fn set_failing(class: Option<&'static str>) {
	for candidate in READ_ERROR_CLASSES {
		metrics::gauge!("d1_usage_failing", "error" => candidate).set(if Some(candidate) == class { 1.0 } else { 0.0 });
	}
}

//! Application-level counters and histograms, recorded through the
//! `metrics` facade per [#139][issue-139] (P1) rather than OpenTelemetry —
//! OTel in this crate is tracing-only from here on; see
//! `metrics::observability` for the span pipeline that remains.
//!
//! [issue-139]: https://github.com/paulgsc/server/issues/139

use metrics::{counter, histogram};
use std::time::Instant;

/// Records operation metrics - replaces timed_operation! macro
pub struct OperationTimer {
	start: Instant,
	operation: String,
	phase: String,
}

impl OperationTimer {
	pub fn new(operation: impl Into<String>, phase: impl Into<String>) -> Self {
		let operation = operation.into();
		let phase = phase.into();

		counter!("operation_count_total", "operation" => operation.clone(), "phase" => phase.clone()).increment(1);

		Self {
			start: Instant::now(),
			operation,
			phase,
		}
	}
}

impl Drop for OperationTimer {
	fn drop(&mut self) {
		let duration = self.start.elapsed().as_secs_f64();
		histogram!("operation_duration_seconds", "operation" => self.operation.clone(), "phase" => self.phase.clone()).record(duration);
	}
}

/// Record cache hit/miss
pub fn record_cache_hit(operation: &str, hit: bool) {
	counter!("cache_hits_total", "operation" => operation.to_string(), "hit" => hit.to_string()).increment(1);
}

/// Record error
pub fn record_error(operation: &str, error_type: &str) {
	counter!("operation_errors_total", "operation" => operation.to_string(), "error_type" => error_type.to_string()).increment(1);
}

/// Record file download with metadata
pub fn record_file_download(file_id: &str, mime_type: &str, size_bytes: usize, cache_type: &str) {
	counter!(
		"file_downloads_total",
		"file_id" => file_id.to_string(),
		"mime_type" => mime_type.to_string(),
		"cache_type" => cache_type.to_string()
	)
	.increment(1);

	histogram!(
		"file_size_bytes",
		"mime_type" => mime_type.to_string(),
		"cache_type" => cache_type.to_string()
	)
	.record(size_bytes as f64);
}

/// Record cache invalidation
pub fn record_cache_invalidation(operation: &str, keys_invalidated: usize) {
	counter!(
		"operation_count_total",
		"operation" => operation.to_string(),
		"phase" => "cache_invalidation",
		"keys_count" => keys_invalidated.to_string()
	)
	.increment(1);
}

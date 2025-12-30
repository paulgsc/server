use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{global, KeyValue};
use std::sync::OnceLock;
use std::time::Instant;

pub struct Metrics {
	pub operation_duration: Histogram<f64>,
	pub operation_count: Counter<u64>,
	pub cache_hits: Counter<u64>,
	pub errors: Counter<u64>,
	pub file_downloads: Counter<u64>,
	pub file_size_bytes: Histogram<f64>,
}

impl Metrics {
	fn get() -> &'static Self {
		static INSTANCE: OnceLock<Metrics> = OnceLock::new();
		INSTANCE.get_or_init(|| {
			let meter = global::meter("file_host");
			Self {
				operation_duration: meter
					.f64_histogram("operation.duration")
					.with_description("Operation duration in seconds")
					.with_unit("s")
					.build(),
				operation_count: meter.u64_counter("operation.count").with_description("Total operations").build(),
				cache_hits: meter.u64_counter("cache.hits").with_description("Cache hit/miss count").build(),
				errors: meter.u64_counter("operation.errors").with_description("Operation errors").build(),
				file_downloads: meter.u64_counter("file.downloads").with_description("Total file downloads").build(),
				file_size_bytes: meter
					.f64_histogram("file.size.bytes")
					.with_description("Downloaded file size in bytes")
					.with_unit("By")
					.build(),
			}
		})
	}
}

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

		Metrics::get()
			.operation_count
			.add(1, &[KeyValue::new("operation", operation.clone()), KeyValue::new("phase", phase.clone())]);

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
		Metrics::get()
			.operation_duration
			.record(duration, &[KeyValue::new("operation", self.operation.clone()), KeyValue::new("phase", self.phase.clone())]);
	}
}

/// Record cache hit/miss
pub fn record_cache_hit(operation: &str, hit: bool) {
	Metrics::get()
		.cache_hits
		.add(1, &[KeyValue::new("operation", operation.to_string()), KeyValue::new("hit", hit.to_string())]);
}

/// Record error
pub fn record_error(operation: &str, error_type: &str) {
	Metrics::get()
		.errors
		.add(1, &[KeyValue::new("operation", operation.to_string()), KeyValue::new("error_type", error_type.to_string())]);
}

/// Record file download with metadata
pub fn record_file_download(file_id: &str, mime_type: &str, size_bytes: usize, cache_type: &str) {
	Metrics::get().file_downloads.add(
		1,
		&[
			KeyValue::new("file_id", file_id.to_string()),
			KeyValue::new("mime_type", mime_type.to_string()),
			KeyValue::new("cache_type", cache_type.to_string()),
		],
	);

	Metrics::get().file_size_bytes.record(
		size_bytes as f64,
		&[KeyValue::new("mime_type", mime_type.to_string()), KeyValue::new("cache_type", cache_type.to_string())],
	);
}

/// Record cache invalidation
pub fn record_cache_invalidation(operation: &str, keys_invalidated: usize) {
	Metrics::get().operation_count.add(
		1,
		&[
			KeyValue::new("operation", operation.to_string()),
			KeyValue::new("phase", "cache_invalidation".to_string()),
			KeyValue::new("keys_count", keys_invalidated.to_string()),
		],
	);
}

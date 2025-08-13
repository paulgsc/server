use log::{info, warn};
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::io::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

use crate::error::FileHostError;

// Instrumentation imports
use prometheus::{
	register_counter, register_gauge, register_histogram, register_int_counter, register_int_gauge, Counter, Error as PrometheusError, Gauge, Histogram, HistogramOpts,
	IntCounter, IntGauge, Opts,
};
use std::sync::LazyLock;
use tracing::{error_span, info_span, instrument, warn_span, Instrument};

/// Cache-specific error types
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
	#[error("Failed to register Prometheus metric: {0}")]
	MetricRegistrationError(#[from] PrometheusError),

	#[error("Cache operation failed: {0}")]
	OperationError(String),

	#[error("Redis connection failed: {0}")]
	RedisConnectionError(#[from] redis::RedisError),

	#[error("Compression/decompression failed: {0}")]
	CompressionError(#[from] std::io::Error),

	#[error("Serialization/deserialization failed: {0}")]
	SerializationError(#[from] serde_json::Error),
}

impl From<CacheError> for FileHostError {
	fn from(cache_error: CacheError) -> Self {
		match cache_error {
			CacheError::RedisConnectionError(redis_err) => FileHostError::RedisError(redis_err),
			CacheError::CompressionError(io_err) => FileHostError::IoError(io_err),
			CacheError::SerializationError(serde_err) => FileHostError::NonSerializableData(serde_err),
			_ => FileHostError::Anyhow(anyhow::anyhow!("Cache error: {}", cache_error)),
		}
	}
}

// Safe metric initialization with error handling
fn register_counter_safe(name: &str, help: &str) -> Result<Counter, CacheError> {
	register_counter!(name, help).map_err(CacheError::MetricRegistrationError)
}

fn register_int_counter_safe(name: &str, help: &str) -> Result<IntCounter, CacheError> {
	register_int_counter!(name, help).map_err(CacheError::MetricRegistrationError)
}

fn register_gauge_safe(name: &str, help: &str) -> Result<Gauge, CacheError> {
	register_gauge!(name, help).map_err(CacheError::MetricRegistrationError)
}

fn register_histogram_safe(opts: HistogramOpts) -> Result<Histogram, CacheError> {
	register_histogram!(opts).map_err(CacheError::MetricRegistrationError)
}

// Prometheus metrics - initialized lazily with error handling
static CACHE_OPERATIONS_TOTAL: LazyLock<Result<Counter, CacheError>> =
	LazyLock::new(|| register_counter_safe("cache_operations_total", "Total number of cache operations by type and result"));

static CACHE_HITS_TOTAL: LazyLock<Result<Counter, CacheError>> = LazyLock::new(|| register_counter_safe("cache_hits_total", "Total number of cache hits by operation type"));

static CACHE_MISSES_TOTAL: LazyLock<Result<IntCounter, CacheError>> = LazyLock::new(|| register_int_counter_safe("cache_misses_total", "Total number of cache misses"));

static CACHE_ERRORS_TOTAL: LazyLock<Result<Counter, CacheError>> = LazyLock::new(|| register_counter_safe("cache_errors_total", "Total number of cache errors by type"));

static CACHE_RETRIES_TOTAL: LazyLock<Result<Counter, CacheError>> = LazyLock::new(|| register_counter_safe("cache_retries_total", "Total number of retry attempts"));

static CACHE_COMPRESSIONS_TOTAL: LazyLock<Result<IntCounter, CacheError>> =
	LazyLock::new(|| register_int_counter_safe("cache_compressions_total", "Total number of compression operations"));

// Performance histograms
static CACHE_OPERATION_DURATION: LazyLock<Result<Histogram, CacheError>> = LazyLock::new(|| {
	register_histogram_safe(
		HistogramOpts::new("cache_operation_duration_seconds", "Duration of cache operations in seconds")
			.buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
	)
});

static CACHE_COMPRESSION_DURATION: LazyLock<Result<Histogram, CacheError>> = LazyLock::new(|| {
	register_histogram_safe(
		HistogramOpts::new("cache_compression_duration_seconds", "Duration of compression operations in seconds")
			.buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1]),
	)
});

static CACHE_DECOMPRESSION_DURATION: LazyLock<Result<Histogram, CacheError>> = LazyLock::new(|| {
	register_histogram_safe(
		HistogramOpts::new("cache_decompression_duration_seconds", "Duration of decompression operations in seconds")
			.buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1]),
	)
});

static CACHE_REDIS_CONNECTION_DURATION: LazyLock<Result<Histogram, CacheError>> = LazyLock::new(|| {
	register_histogram_safe(
		HistogramOpts::new("cache_redis_connection_duration_seconds", "Duration to establish Redis connection in seconds")
			.buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5]),
	)
});

// Size metrics
static CACHE_DATA_SIZE: LazyLock<Result<Histogram, CacheError>> = LazyLock::new(|| {
	register_histogram_safe(
		HistogramOpts::new("cache_data_size_bytes", "Size of cached data in bytes").buckets(vec![1024.0, 4096.0, 16384.0, 65536.0, 262144.0, 1048576.0, 4194304.0, 16777216.0]),
	)
});

static CACHE_COMPRESSED_SIZE: LazyLock<Result<Histogram, CacheError>> = LazyLock::new(|| {
	register_histogram_safe(
		HistogramOpts::new("cache_compressed_size_bytes", "Size of compressed data in bytes")
			.buckets(vec![512.0, 2048.0, 8192.0, 32768.0, 131072.0, 524288.0, 2097152.0, 8388608.0]),
	)
});

static CACHE_COMPRESSION_RATIO: LazyLock<Result<Gauge, CacheError>> =
	LazyLock::new(|| register_gauge_safe("cache_compression_ratio", "Compression ratio (original_size / compressed_size)"));

// TTL and access patterns
static CACHE_TTL_SECONDS: LazyLock<Result<Histogram, CacheError>> = LazyLock::new(|| {
	register_histogram_safe(HistogramOpts::new("cache_ttl_seconds", "TTL values set for cache entries").buckets(vec![60.0, 300.0, 900.0, 3600.0, 10800.0, 86400.0, 604800.0]))
});

static CACHE_ACCESS_COUNT: LazyLock<Result<Histogram, CacheError>> = LazyLock::new(|| {
	register_histogram_safe(
		HistogramOpts::new("cache_access_count", "Number of times entries have been accessed").buckets(vec![1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0]),
	)
});

static CACHE_ENTRY_AGE: LazyLock<Result<Gauge, CacheError>> = LazyLock::new(|| register_gauge_safe("cache_entry_age_seconds", "Age of cache entries when accessed"));

// Exported macro for recording cache operation metrics
#[macro_export]
macro_rules! record_cache_operation {
	($operation:expr, $result:expr, $duration:expr) => {
		if let Ok(counter) = &*$crate::CACHE_OPERATIONS_TOTAL {
			counter
				.with_label_values(&[
					$operation,
					match $result {
						Ok(_) => "success",
						Err(_) => "error",
					},
				])
				.inc();
		}

		if let Ok(histogram) = &*$crate::CACHE_OPERATION_DURATION {
			histogram.with_label_values(&[$operation]).observe($duration.as_secs_f64());
		}
	};
}

// Exported macro for recording cache hits/misses
#[macro_export]
macro_rules! record_cache_access {
	(hit, $operation:expr, $access_count:expr, $entry_age:expr) => {
		if let Ok(counter) = &*$crate::CACHE_HITS_TOTAL {
			counter.with_label_values(&[$operation]).inc();
		}

		if let Ok(histogram) = &*$crate::CACHE_ACCESS_COUNT {
			histogram.observe($access_count as f64);
		}

		if let Ok(gauge) = &*$crate::CACHE_ENTRY_AGE {
			gauge.set($entry_age as f64);
		}
	};
	(miss, $operation:expr) => {
		if let Ok(counter) = &*$crate::CACHE_MISSES_TOTAL {
			counter.inc();
		}
	};
}

// Exported macro for recording compression metrics
#[macro_export]
macro_rules! record_compression {
	($original_size:expr, $compressed_size:expr, $duration:expr) => {
		if let Ok(counter) = &*$crate::CACHE_COMPRESSIONS_TOTAL {
			counter.inc();
		}

		if let Ok(histogram) = &*$crate::CACHE_DATA_SIZE {
			histogram.observe($original_size as f64);
		}

		if let Ok(histogram) = &*$crate::CACHE_COMPRESSED_SIZE {
			histogram.observe($compressed_size as f64);
		}

		if let Ok(histogram) = &*$crate::CACHE_COMPRESSION_DURATION {
			histogram.observe($duration.as_secs_f64());
		}

		if $compressed_size > 0 {
			if let Ok(gauge) = &*$crate::CACHE_COMPRESSION_RATIO {
				let ratio = $original_size as f64 / $compressed_size as f64;
				gauge.set(ratio);
			}
		}
	};
}

// Exported macro for recording retry attempts
#[macro_export]
macro_rules! record_retry {
	($operation:expr, $attempt:expr, $error_type:expr) => {
		if let Ok(counter) = &*$crate::CACHE_RETRIES_TOTAL {
			counter.with_label_values(&[$operation, &$attempt.to_string()]).inc();
		}

		if let Ok(counter) = &*$crate::CACHE_ERRORS_TOTAL {
			counter.with_label_values(&[$operation, $error_type]).inc();
		}
	};
}

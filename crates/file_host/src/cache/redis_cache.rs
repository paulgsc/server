use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use log::{info, warn};
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::{
	io::{Read, Write},
	sync::Arc,
	time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::time::sleep;

use crate::Config;

// Instrumentation imports
use prometheus::{
	register_counter_vec, register_gauge, register_histogram_vec, register_int_counter, CounterVec, Error as PrometheusError, Gauge, HistogramOpts, HistogramVec, IntCounter,
};
use std::sync::LazyLock;
use tracing::instrument;

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

	#[error("System time error: {0}")]
	SystemTimeError(#[from] std::time::SystemTimeError),

	#[error("Integer conversion error: {0}")]
	TryFromIntError(#[from] std::num::TryFromIntError),

	#[error("Generic error: {0}")]
	Generic(#[from] anyhow::Error),
}

// Safe metric initialization with error handling
fn register_counter_vec_safe(name: &str, help: &str, labels: &[&str]) -> Result<CounterVec, CacheError> {
	register_counter_vec!(name, help, labels).map_err(CacheError::MetricRegistrationError)
}

fn register_int_counter_safe(name: &str, help: &str) -> Result<IntCounter, CacheError> {
	register_int_counter!(name, help).map_err(CacheError::MetricRegistrationError)
}

fn register_gauge_safe(name: &str, help: &str) -> Result<Gauge, CacheError> {
	register_gauge!(name, help).map_err(CacheError::MetricRegistrationError)
}

fn register_histogram_vec_safe(opts: HistogramOpts, labels: &[&str]) -> Result<HistogramVec, CacheError> {
	register_histogram_vec!(opts, labels).map_err(CacheError::MetricRegistrationError)
}

// Prometheus metrics - initialized lazily with error handling
static CACHE_OPERATIONS_TOTAL: LazyLock<Result<CounterVec, CacheError>> =
	LazyLock::new(|| register_counter_vec_safe("cache_operations_total", "Total number of cache operations by type and result", &["operation", "result"]));

static CACHE_HITS_TOTAL: LazyLock<Result<CounterVec, CacheError>> =
	LazyLock::new(|| register_counter_vec_safe("cache_hits_total", "Total number of cache hits by operation type", &["operation"]));

static CACHE_MISSES_TOTAL: LazyLock<Result<IntCounter, CacheError>> = LazyLock::new(|| register_int_counter_safe("cache_misses_total", "Total number of cache misses"));

static CACHE_ERRORS_TOTAL: LazyLock<Result<CounterVec, CacheError>> =
	LazyLock::new(|| register_counter_vec_safe("cache_errors_total", "Total number of cache errors by type", &["operation", "error_type"]));

static CACHE_RETRIES_TOTAL: LazyLock<Result<CounterVec, CacheError>> =
	LazyLock::new(|| register_counter_vec_safe("cache_retries_total", "Total number of retry attempts", &["operation", "attempt"]));

static CACHE_COMPRESSIONS_TOTAL: LazyLock<Result<IntCounter, CacheError>> =
	LazyLock::new(|| register_int_counter_safe("cache_compressions_total", "Total number of compression operations"));

// Performance histograms
static CACHE_OPERATION_DURATION: LazyLock<Result<HistogramVec, CacheError>> = LazyLock::new(|| {
	register_histogram_vec_safe(
		HistogramOpts::new("cache_operation_duration_seconds", "Duration of cache operations in seconds")
			.buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
		&["operation"],
	)
});

static CACHE_COMPRESSION_DURATION: LazyLock<Result<HistogramVec, CacheError>> = LazyLock::new(|| {
	register_histogram_vec_safe(
		HistogramOpts::new("cache_compression_duration_seconds", "Duration of compression operations in seconds")
			.buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1]),
		&["operation"],
	)
});

static CACHE_DECOMPRESSION_DURATION: LazyLock<Result<HistogramVec, CacheError>> = LazyLock::new(|| {
	register_histogram_vec_safe(
		HistogramOpts::new("cache_decompression_duration_seconds", "Duration of decompression operations in seconds")
			.buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1]),
		&["operation"],
	)
});

static CACHE_REDIS_CONNECTION_DURATION: LazyLock<Result<HistogramVec, CacheError>> = LazyLock::new(|| {
	register_histogram_vec_safe(
		HistogramOpts::new("cache_redis_connection_duration_seconds", "Duration to establish Redis connection in seconds")
			.buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5]),
		&["operation"],
	)
});

// Size metrics
static CACHE_DATA_SIZE: LazyLock<Result<HistogramVec, CacheError>> = LazyLock::new(|| {
	register_histogram_vec_safe(
		HistogramOpts::new("cache_data_size_bytes", "Size of cached data in bytes").buckets(vec![1024.0, 4096.0, 16384.0, 65536.0, 262144.0, 1048576.0, 4194304.0, 16777216.0]),
		&["operation"],
	)
});

static CACHE_COMPRESSED_SIZE: LazyLock<Result<HistogramVec, CacheError>> = LazyLock::new(|| {
	register_histogram_vec_safe(
		HistogramOpts::new("cache_compressed_size_bytes", "Size of compressed data in bytes")
			.buckets(vec![512.0, 2048.0, 8192.0, 32768.0, 131072.0, 524288.0, 2097152.0, 8388608.0]),
		&["operation"],
	)
});

static CACHE_COMPRESSION_RATIO: LazyLock<Result<Gauge, CacheError>> =
	LazyLock::new(|| register_gauge_safe("cache_compression_ratio", "Compression ratio (original_size / compressed_size)"));

// TTL and access patterns
static CACHE_TTL_SECONDS: LazyLock<Result<HistogramVec, CacheError>> = LazyLock::new(|| {
	register_histogram_vec_safe(
		HistogramOpts::new("cache_ttl_seconds", "TTL values set for cache entries").buckets(vec![60.0, 300.0, 900.0, 3600.0, 10800.0, 86400.0, 604800.0]),
		&["operation"],
	)
});

static CACHE_ACCESS_COUNT: LazyLock<Result<HistogramVec, CacheError>> = LazyLock::new(|| {
	register_histogram_vec_safe(
		HistogramOpts::new("cache_access_count", "Number of times entries have been accessed").buckets(vec![1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0]),
		&["operation"],
	)
});

static CACHE_ENTRY_AGE: LazyLock<Result<Gauge, CacheError>> = LazyLock::new(|| register_gauge_safe("cache_entry_age_seconds", "Age of cache entries when accessed"));

// Cache entry with metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntry<T> {
	pub data: T,
	pub content_type: Option<String>,
	pub created_at: u64,
	pub ttl: u64,
	pub access_count: u64,
	pub last_accessed: u64,
}

impl<T> CacheEntry<T> {
	pub fn new(data: T, ttl: u64) -> Result<Self, CacheError> {
		let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

		// Record TTL metric
		if let Ok(histogram) = &*CACHE_TTL_SECONDS {
			histogram.with_label_values(&["new"]).observe(ttl as f64);
		}

		Ok(Self {
			data,
			content_type: None,
			created_at: now,
			ttl,
			access_count: 1,
			last_accessed: now,
		})
	}

	pub fn with_content_type(mut self, content_type: String) -> Self {
		self.content_type = Some(content_type);
		self
	}

	pub fn touch(&mut self) -> Result<(), CacheError> {
		self.access_count += 1;
		self.last_accessed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
		Ok(())
	}

	pub fn age_seconds(&self) -> u64 {
		SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map(|dur| dur.as_secs().saturating_sub(self.created_at))
			.unwrap_or(0)
	}
}

// Cache configuration
#[derive(Clone, Debug)]
pub struct CacheConfig {
	pub redis_url: String,
	pub default_ttl: u64,
	pub max_retries: u32,
	pub retry_delay_ms: u64,
	pub enable_compression: bool,
	pub compression_threshold: usize, // Compress if data > this size
	pub key_prefix: String,
}

impl Default for CacheConfig {
	fn default() -> Self {
		Self {
			redis_url: "redis://127.0.0.1:6379".to_string(),
			default_ttl: 3600, // 1 hour
			max_retries: 3,
			retry_delay_ms: 100,
			enable_compression: true,
			compression_threshold: 1024, // 1KB
			key_prefix: "cache:".to_string(),
		}
	}
}

impl From<Arc<Config>> for CacheConfig {
	fn from(cfg: Arc<Config>) -> Self {
		Self {
			redis_url: cfg.redis_url.clone().unwrap_or_else(|| "redis://127.0.0.1:6379".into()),
			default_ttl: cfg.cache_ttl,
			max_retries: 3,      // you can add these as new Config args if you want
			retry_delay_ms: 100, // same here
			enable_compression: true,
			compression_threshold: 1024,
			key_prefix: "cache:".into(),
		}
	}
}

// Cache store
#[derive(Clone)]
pub struct CacheStore {
	redis_client: Client,
	config: CacheConfig,
}

impl CacheStore {
	pub fn new(config: CacheConfig) -> Result<Self, CacheError> {
		let redis_client = Client::open(config.redis_url.as_str())?;
		Ok(Self { redis_client, config })
	}

	// Generate prefixed key
	fn make_key(&self, key: &str) -> String {
		format!("{}{}", self.config.key_prefix, key)
	}

	// Retry mechanism for Redis operations with instrumentation
	async fn with_retry<F, T>(&self, operation_name: &str, mut operation: F) -> Result<T, CacheError>
	where
		F: FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, CacheError>> + Send>>,
	{
		let start_time = Instant::now();
		let mut last_error = None;

		for attempt in 0..=self.config.max_retries {
			match operation().await {
				Ok(result) => {
					let duration = start_time.elapsed();

					// Record successful operation
					if let Ok(counter) = &*CACHE_OPERATIONS_TOTAL {
						counter.with_label_values(&[operation_name, "success"]).inc();
					}
					if let Ok(histogram) = &*CACHE_OPERATION_DURATION {
						histogram.with_label_values(&[operation_name]).observe(duration.as_secs_f64());
					}

					return Ok(result);
				}
				Err(e) => {
					last_error = Some(e);

					// Record retry attempt
					if let Ok(counter) = &*CACHE_RETRIES_TOTAL {
						counter.with_label_values(&[operation_name, &attempt.to_string()]).inc();
					}
					if let Ok(counter) = &*CACHE_ERRORS_TOTAL {
						counter.with_label_values(&[operation_name, "redis_error"]).inc();
					}

					if attempt < self.config.max_retries {
						warn!("Cache operation {} failed (attempt {}), retrying...", operation_name, attempt + 1);
						sleep(Duration::from_millis(self.config.retry_delay_ms * (attempt as u64 + 1))).await;
					}
				}
			}
		}

		let duration = start_time.elapsed();

		// Record final failure
		if let Ok(counter) = &*CACHE_OPERATIONS_TOTAL {
			counter.with_label_values(&[operation_name, "error"]).inc();
		}
		if let Ok(histogram) = &*CACHE_OPERATION_DURATION {
			histogram.with_label_values(&[operation_name]).observe(duration.as_secs_f64());
		}

		Err(last_error.unwrap())
	}

	// Compression helpers with instrumentation
	fn compress_data(&self, data: &[u8]) -> Result<Vec<u8>, CacheError> {
		if !self.config.enable_compression || data.len() < self.config.compression_threshold {
			return Ok(data.to_vec());
		}

		let start_time = Instant::now();
		let original_size = data.len();

		let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
		encoder.write_all(data)?;
		let compressed = encoder.finish()?;

		let duration = start_time.elapsed();
		let compressed_size = compressed.len();

		// Record compression metrics
		if let Ok(counter) = &*CACHE_COMPRESSIONS_TOTAL {
			counter.inc();
		}
		if let Ok(histogram) = &*CACHE_DATA_SIZE {
			histogram.with_label_values(&["compress"]).observe(original_size as f64);
		}
		if let Ok(histogram) = &*CACHE_COMPRESSED_SIZE {
			histogram.with_label_values(&["compress"]).observe(compressed_size as f64);
		}
		if let Ok(histogram) = &*CACHE_COMPRESSION_DURATION {
			histogram.with_label_values(&["compress"]).observe(duration.as_secs_f64());
		}
		if compressed_size > 0 {
			if let Ok(gauge) = &*CACHE_COMPRESSION_RATIO {
				let ratio = original_size as f64 / compressed_size as f64;
				gauge.set(ratio);
			}
		}

		Ok(compressed)
	}

	fn decompress_data(&self, data: &[u8]) -> Result<Vec<u8>, CacheError> {
		if !self.config.enable_compression {
			return Ok(data.to_vec());
		}

		let start_time = Instant::now();

		// Try to decompress, fallback to original data if it fails (wasn't compressed)
		let mut decoder = GzDecoder::new(data);
		let mut decompressed = Vec::new();
		let result = match decoder.read_to_end(&mut decompressed) {
			Ok(_) => Ok(decompressed),
			Err(_) => Ok(data.to_vec()), // Assume it wasn't compressed
		};

		let duration = start_time.elapsed();
		if let Ok(histogram) = &*CACHE_DECOMPRESSION_DURATION {
			histogram.with_label_values(&["decompress"]).observe(duration.as_secs_f64());
		}

		result
	}

	// Generic set method with compression and metadata
	#[instrument(skip(self, data), fields(key = %key, ttl = ?ttl))]
	pub async fn set<T: Serialize>(&self, key: &str, data: &T, ttl: Option<u64>) -> Result<(), CacheError> {
		let cache_key = self.make_key(key);
		let ttl = ttl.unwrap_or(self.config.default_ttl);

		let entry = CacheEntry::new(data, ttl)?;
		let serialized = serde_json::to_vec(&entry)?;
		let compressed = self.compress_data(&serialized)?;

		self
			.with_retry("set", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();
				let compressed = compressed.clone();
				let ttl = ttl;

				Box::pin(async move {
					let connection_start = Instant::now();
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let connection_duration = connection_start.elapsed();

					if let Ok(histogram) = &*CACHE_REDIS_CONNECTION_DURATION {
						histogram.with_label_values(&["set"]).observe(connection_duration.as_secs_f64());
					}

					let _: () = con.set_ex(&cache_key, compressed, ttl).await?;
					Result::<_, CacheError>::Ok(())
				})
			})
			.await?;

		info!("Cached data: {} (TTL: {}s)", key, ttl);
		Ok(())
	}

	// Generic get method with decompression and touch
	#[instrument(skip(self), fields(key = %key))]
	pub async fn get<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>, CacheError> {
		let cache_key = self.make_key(key);

		let data: Option<Vec<u8>> = self
			.with_retry("get", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let connection_start = Instant::now();
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let connection_duration = connection_start.elapsed();

					if let Ok(histogram) = &*CACHE_REDIS_CONNECTION_DURATION {
						histogram.with_label_values(&["get"]).observe(connection_duration.as_secs_f64());
					}

					let result: Option<Vec<u8>> = con.get(&cache_key).await?;
					Result::<_, CacheError>::Ok(result)
				})
			})
			.await?;

		match data {
			Some(compressed_data) => {
				let decompressed = self.decompress_data(&compressed_data)?;
				let mut entry: CacheEntry<T> = serde_json::from_slice(&decompressed)?;

				// Touch the entry (update access info)
				entry.touch()?;
				let entry_age = entry.age_seconds();
				let access_count = entry.access_count;

				// Record cache hit
				if let Ok(counter) = &*CACHE_HITS_TOTAL {
					counter.with_label_values(&["get"]).inc();
				}
				if let Ok(histogram) = &*CACHE_ACCESS_COUNT {
					histogram.with_label_values(&["get"]).observe(access_count as f64);
				}
				if let Ok(gauge) = &*CACHE_ENTRY_AGE {
					gauge.set(entry_age as f64);
				}

				// Update Redis with new access info (fire and forget)
				let _ = self.touch_entry(key).await.map_err(|e| {
					warn!("Failed to touch cache entry {}: {}", key, e);
					e
				});

				info!("Cache hit: {} (accessed {} times, age: {}s)", key, entry.access_count, entry_age);
				Ok(Some(entry.data))
			}
			None => {
				// Record cache miss
				if let Ok(counter) = &*CACHE_MISSES_TOTAL {
					counter.inc();
				}

				Ok(None)
			}
		}
	}

	// Specialized method for binary data (audio, images, etc.)
	#[instrument(skip(self, data), fields(key = %key, data_size = data.len(), content_type = ?content_type, ttl = ?ttl))]
	pub async fn set_binary(&self, key: &str, data: &[u8], content_type: Option<String>, ttl: Option<u64>) -> Result<(), CacheError> {
		let cache_key = self.make_key(key);
		let ttl = ttl.unwrap_or(self.config.default_ttl);

		let mut entry = CacheEntry::new(data.to_vec(), ttl)?;
		if let Some(ct) = content_type {
			entry = entry.with_content_type(ct);
		}

		let serialized = serde_json::to_vec(&entry)?;
		let compressed = self.compress_data(&serialized)?;

		self
			.with_retry("set_binary", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();
				let compressed = compressed.clone();
				let ttl = ttl;

				Box::pin(async move {
					let connection_start = Instant::now();
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let connection_duration = connection_start.elapsed();

					if let Ok(histogram) = &*CACHE_REDIS_CONNECTION_DURATION {
						histogram.with_label_values(&["set_binary"]).observe(connection_duration.as_secs_f64());
					}

					let _: () = con.set_ex(&cache_key, compressed, ttl).await?;
					Result::<_, CacheError>::Ok(())
				})
			})
			.await?;

		info!("Cached binary data: {} ({} bytes, TTL: {}s)", key, data.len(), ttl);
		Ok(())
	}

	// Get binary data with metadata
	#[instrument(skip(self), fields(key = %key))]
	pub async fn get_binary(&self, key: &str) -> Result<Option<(Vec<u8>, Option<String>)>, CacheError> {
		let cache_key = self.make_key(key);

		let data: Option<Vec<u8>> = self
			.with_retry("get_binary", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let connection_start = Instant::now();
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let connection_duration = connection_start.elapsed();

					if let Ok(histogram) = &*CACHE_REDIS_CONNECTION_DURATION {
						histogram.with_label_values(&["get_binary"]).observe(connection_duration.as_secs_f64());
					}

					let result: Option<Vec<u8>> = con.get(&cache_key).await?;
					Result::<_, CacheError>::Ok(result)
				})
			})
			.await?;

		match data {
			Some(compressed_data) => {
				let decompressed = self.decompress_data(&compressed_data)?;
				let mut entry: CacheEntry<Vec<u8>> = serde_json::from_slice(&decompressed)?;

				entry.touch()?;
				let entry_age = entry.age_seconds();
				let access_count = entry.access_count;

				// Record cache hit
				if let Ok(counter) = &*CACHE_HITS_TOTAL {
					counter.with_label_values(&["get_binary"]).inc();
				}
				if let Ok(histogram) = &*CACHE_ACCESS_COUNT {
					histogram.with_label_values(&["get_binary"]).observe(access_count as f64);
				}
				if let Ok(gauge) = &*CACHE_ENTRY_AGE {
					gauge.set(entry_age as f64);
				}

				let _ = self.touch_entry(key).await.map_err(|e| {
					warn!("Failed to touch binary cache entry {}: {}", key, e);
					e
				});

				info!(
					"Binary cache hit: {} ({} bytes, accessed {} times, age: {}s)",
					key,
					entry.data.len(),
					access_count,
					entry_age
				);
				Ok(Some((entry.data, entry.content_type)))
			}
			None => {
				// Record cache miss
				if let Ok(counter) = &*CACHE_MISSES_TOTAL {
					counter.inc();
				}

				Ok(None)
			}
		}
	}

	// Touch an entry (update access metadata)
	async fn touch_entry(&self, key: &str) -> Result<(), CacheError> {
		let cache_key = self.make_key(key);
		let ttl = self.config.default_ttl;

		self
			.with_retry("touch", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let connection_start = Instant::now();
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let connection_duration = connection_start.elapsed();

					if let Ok(histogram) = &*CACHE_REDIS_CONNECTION_DURATION {
						histogram.with_label_values(&["touch"]).observe(connection_duration.as_secs_f64());
					}

					let _: () = con.expire(&cache_key, ttl.try_into()?).await?;
					Result::<_, CacheError>::Ok(())
				})
			})
			.await?;

		Ok(())
	}

	// Delete specific key
	#[instrument(skip(self), fields(key = %key))]
	pub async fn delete(&self, key: &str) -> Result<bool, CacheError> {
		let cache_key = self.make_key(key);

		let deleted: i32 = self
			.with_retry("delete", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let connection_start = Instant::now();
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let connection_duration = connection_start.elapsed();

					if let Ok(histogram) = &*CACHE_REDIS_CONNECTION_DURATION {
						histogram.with_label_values(&["delete"]).observe(connection_duration.as_secs_f64());
					}

					let result: i32 = con.del(&cache_key).await?;
					Result::<_, CacheError>::Ok(result)
				})
			})
			.await?;

		Ok(deleted > 0)
	}

	// Flush all cache entries with the configured prefix
	#[instrument(skip(self))]
	pub async fn flush_all(&self) -> Result<u64, CacheError> {
		let pattern = format!("{}*", self.config.key_prefix);

		let deleted: u64 = self
			.with_retry("flush_all", || {
				let redis_client = self.redis_client.clone();
				let pattern = pattern.clone();

				Box::pin(async move {
					let connection_start = Instant::now();
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let connection_duration = connection_start.elapsed();

					if let Ok(histogram) = &*CACHE_REDIS_CONNECTION_DURATION {
						histogram.with_label_values(&["flush_all"]).observe(connection_duration.as_secs_f64());
					}

					// Get all keys matching pattern
					let keys: Vec<String> = con.keys(&pattern).await?;
					if keys.is_empty() {
						return Ok(0);
					}

					// Delete all matching keys
					let deleted: i32 = con.del(&keys).await?;
					Result::<_, CacheError>::Ok(deleted as u64)
				})
			})
			.await?;

		info!("Flushed {} cache entries", deleted);
		Ok(deleted)
	}
}

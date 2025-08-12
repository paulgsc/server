use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use log::{info, warn};
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

use crate::error::FileHostError;

//  cache entry with metadata
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
	pub fn new(data: T, ttl: u64) -> Result<Self, FileHostError> {
		let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| FileHostError::Anyhow(e.into()))?.as_secs();

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

	pub fn touch(&mut self) -> Result<(), FileHostError> {
		self.access_count += 1;
		self.last_accessed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| FileHostError::Anyhow(e.into()))?.as_secs();
		Ok(())
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

//  cache store
#[derive(Clone)]
pub struct CacheStore {
	redis_client: Client,
	config: CacheConfig,
}

impl CacheStore {
	pub fn new(config: CacheConfig) -> Result<Self, FileHostError> {
		let redis_client = Client::open(config.redis_url.as_str())?;
		Ok(Self { redis_client, config })
	}

	// Generate prefixed key
	fn make_key(&self, key: &str) -> String {
		format!("{}{}", self.config.key_prefix, key)
	}

	// Retry mechanism for Redis operations, now returning a concrete FileHostError
	async fn with_retry<F, T>(&self, mut operation: F) -> Result<T, FileHostError>
	where
		F: FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, FileHostError>> + Send>>,
	{
		let mut last_error = None;

		for attempt in 0..=self.config.max_retries {
			match operation().await {
				Ok(result) => return Ok(result),
				Err(e) => {
					last_error = Some(e);
					if attempt < self.config.max_retries {
						warn!("Cache operation failed (attempt {}), retrying...", attempt + 1);
						sleep(Duration::from_millis(self.config.retry_delay_ms * (attempt as u64 + 1))).await;
					}
				}
			}
		}

		Err(last_error.unwrap())
	}

	// Compression helpers
	fn compress_data(&self, data: &[u8]) -> Result<Vec<u8>, FileHostError> {
		if !self.config.enable_compression || data.len() < self.config.compression_threshold {
			return Ok(data.to_vec());
		}

		let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
		encoder.write_all(data)?;
		Ok(encoder.finish()?)
	}

	fn decompress_data(&self, data: &[u8]) -> Result<Vec<u8>, FileHostError> {
		if !self.config.enable_compression {
			return Ok(data.to_vec());
		}

		// Try to decompress, fallback to original data if it fails (wasn't compressed)
		let mut decoder = GzDecoder::new(data);
		let mut decompressed = Vec::new();
		match decoder.read_to_end(&mut decompressed) {
			Ok(_) => Ok(decompressed),
			Err(_) => Ok(data.to_vec()), // Assume it wasn't compressed
		}
	}

	// Generic set method with compression and metadata
	pub async fn set<T: Serialize>(&self, key: &str, data: &T, ttl: Option<u64>) -> Result<(), FileHostError> {
		let cache_key = self.make_key(key);
		let ttl = ttl.unwrap_or(self.config.default_ttl);

		let entry = CacheEntry::new(data, ttl)?;
		let serialized = serde_json::to_vec(&entry)?;
		let compressed = self.compress_data(&serialized)?;

		self
			.with_retry(|| {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();
				let compressed = compressed.clone();
				let ttl = ttl;

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let _: () = con.set_ex(&cache_key, compressed, ttl).await?;
					Result::<_, FileHostError>::Ok(())
				})
			})
			.await?;

		info!("Cached data: {} (TTL: {}s)", key, ttl);
		Ok(())
	}

	// Generic get method with decompression and touch
	pub async fn get<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>, FileHostError> {
		let cache_key = self.make_key(key);

		let data: Option<Vec<u8>> = self
			.with_retry(|| {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let result: Option<Vec<u8>> = con.get(&cache_key).await?;
					Result::<_, FileHostError>::Ok(result)
				})
			})
			.await?;

		match data {
			Some(compressed_data) => {
				let decompressed = self.decompress_data(&compressed_data)?;
				let mut entry: CacheEntry<T> = serde_json::from_slice(&decompressed)?;

				// Touch the entry (update access info)
				entry.touch()?;
				self.touch_entry(key).await.map_err(|e| warn!("Failed to touch cache entry {}: {}", key, e)).ok(); // Log the error instead of silently failing

				info!("Cache hit: {} (accessed {} times)", key, entry.access_count);
				Ok(Some(entry.data))
			}
			None => Ok(None),
		}
	}

	// Specialized method for binary data (audio, images, etc.)
	pub async fn set_binary(&self, key: &str, data: &[u8], content_type: Option<String>, ttl: Option<u64>) -> Result<(), FileHostError> {
		let cache_key = self.make_key(key);
		let ttl = ttl.unwrap_or(self.config.default_ttl);

		let mut entry = CacheEntry::new(data.to_vec(), ttl)?;
		if let Some(ct) = content_type {
			entry = entry.with_content_type(ct);
		}

		let serialized = serde_json::to_vec(&entry)?;
		let compressed = self.compress_data(&serialized)?;

		self
			.with_retry(|| {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();
				let compressed = compressed.clone();
				let ttl = ttl;

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let _: () = con.set_ex(&cache_key, compressed, ttl).await?;
					Result::<_, FileHostError>::Ok(())
				})
			})
			.await?;

		info!("Cached binary data: {} ({} bytes, TTL: {}s)", key, data.len(), ttl);
		Ok(())
	}

	// Get binary data with metadata
	pub async fn get_binary(&self, key: &str) -> Result<Option<(Vec<u8>, Option<String>)>, FileHostError> {
		let cache_key = self.make_key(key);

		let data: Option<Vec<u8>> = self
			.with_retry(|| {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let result: Option<Vec<u8>> = con.get(&cache_key).await?;
					Result::<_, FileHostError>::Ok(result)
				})
			})
			.await?;

		match data {
			Some(compressed_data) => {
				let decompressed = self.decompress_data(&compressed_data)?;
				let mut entry: CacheEntry<Vec<u8>> = serde_json::from_slice(&decompressed)?;

				entry.touch()?;
				self.touch_entry(key).await.map_err(|e| warn!("Failed to touch binary cache entry {}: {}", key, e)).ok();

				info!("Binary cache hit: {} ({} bytes)", key, entry.data.len());
				Ok(Some((entry.data, entry.content_type)))
			}
			None => Ok(None),
		}
	}

	// Touch an entry (update access metadata)
	async fn touch_entry(&self, key: &str) -> Result<(), FileHostError> {
		let cache_key = self.make_key(key);
		let ttl = self.config.default_ttl;

		self
			.with_retry(|| {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let _: () = con.expire(&cache_key, ttl.try_into()?).await?;
					Result::<_, FileHostError>::Ok(())
				})
			})
			.await?;

		Ok(())
	}

	// Delete specific key
	pub async fn delete(&self, key: &str) -> Result<bool, FileHostError> {
		let cache_key = self.make_key(key);

		let deleted: i32 = self
			.with_retry(|| {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let result: i32 = con.del(&cache_key).await?;
					Result::<_, FileHostError>::Ok(result)
				})
			})
			.await?;

		Ok(deleted > 0)
	}

	// Flush all cache entries with the configured prefix
	pub async fn flush_all(&self) -> Result<u64, FileHostError> {
		let pattern = format!("{}*", self.config.key_prefix);

		let deleted: u64 = self
			.with_retry(|| {
				let redis_client = self.redis_client.clone();
				let pattern = pattern.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;

					// Get all keys matching pattern
					let keys: Vec<String> = con.keys(&pattern).await?;
					if keys.is_empty() {
						return Ok(0);
					}

					// Delete all matching keys
					let deleted: i32 = con.del(&keys).await?;
					Result::<_, FileHostError>::Ok(deleted as u64)
				})
			})
			.await?;

		info!("Flushed {} cache entries", deleted);
		Ok(deleted)
	}
}

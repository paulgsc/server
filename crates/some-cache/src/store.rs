use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Duration};
use tracing::{info, instrument, warn};
use zstd::stream::{decode_all, encode_all};

use crate::{
	config::CacheConfig,
	entry::{BinaryCacheEntry, CacheEntry},
	error::CacheError,
	metrics::{namespace_of, CACHE_HITS, CACHE_MISSES},
};

// ── Payload encoding ──────────────────────────────────────────────────────────
//
// Leading discriminant byte:
//   0x00 → raw (no compression)
//   0x01 → zstd-compressed

const FLAG_RAW: u8 = 0x00;
const FLAG_ZSTD: u8 = 0x01;

// ── CacheStore ────────────────────────────────────────────────────────────────

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

	pub fn redis_client(&self) -> &Client {
		&self.redis_client
	}

	pub fn config(&self) -> &CacheConfig {
		&self.config
	}

	// ── Key helpers ───────────────────────────────────────────────────────

	fn make_key(&self, key: &str) -> String {
		format!("{}{}", self.config.key_prefix, key)
	}

	// ── Payload encoding ──────────────────────────────────────────────────

	fn encode_payload(&self, data: &[u8]) -> Result<Vec<u8>, CacheError> {
		let should_compress = self.config.enable_compression && data.len() >= self.config.compression_threshold;

		if !should_compress {
			let mut out = Vec::with_capacity(1 + data.len());
			out.push(FLAG_RAW);
			out.extend_from_slice(data);
			return Ok(out);
		}

		let level = self.config.zstd_level.unwrap_or(3);
		let compressed = encode_all(data, level).map_err(CacheError::Compression)?;

		let mut out = Vec::with_capacity(1 + compressed.len());
		out.push(FLAG_ZSTD);
		out.extend_from_slice(&compressed);
		Ok(out)
	}

	fn decode_payload(&self, data: &[u8]) -> Result<Vec<u8>, CacheError> {
		match data.split_first() {
			None => Ok(Vec::new()),
			Some((&FLAG_RAW, rest)) => Ok(rest.to_vec()),
			Some((&FLAG_ZSTD, rest)) => decode_all(rest).map_err(CacheError::Decompression),
			Some((&flag, _)) => Err(CacheError::InvalidEncoding(flag)),
		}
	}

	// ── Retry ─────────────────────────────────────────────────────────────

	async fn with_retry<F, T>(&self, operation_name: &str, mut operation: F) -> Result<T, CacheError>
	where
		F: FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, CacheError>> + Send>>,
	{
		let mut last_error = None;

		for attempt in 0..=self.config.max_retries {
			match operation().await {
				Ok(result) => return Ok(result),
				Err(e) => {
					last_error = Some(e);
					if attempt < self.config.max_retries {
						warn!("Cache operation {} failed (attempt {}), retrying…", operation_name, attempt + 1);
						let backoff_ms = self.config.retry_delay_ms * (attempt as u64 + 1);
						sleep(Duration::from_millis(backoff_ms)).await;
					}
				}
			}
		}

		Err(last_error.unwrap())
	}

	// ── TTL refresh ───────────────────────────────────────────────────────

	fn should_touch(&self) -> bool {
		let p = self.config.touch_probability.unwrap_or(0.1);
		if p <= 0.0 {
			return false;
		}
		if p >= 1.0 {
			return true;
		}
		fastrand::f64() < p
	}

	async fn touch_entry(&self, cache_key: &str, ttl: u64) -> Result<(), CacheError> {
		self
			.with_retry("touch", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.to_string();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let _: () = con.expire(&cache_key, ttl.try_into()?).await?;
					Result::<_, CacheError>::Ok(())
				})
			})
			.await
	}

	// ── Public API ────────────────────────────────────────────────────────

	#[instrument(skip(self, data), fields(key = %key))]
	pub async fn set<T: Serialize>(&self, key: &str, data: &T, ttl: Option<u64>) -> Result<(), CacheError> {
		let cache_key = self.make_key(key);
		let ttl = ttl.unwrap_or(self.config.default_ttl);

		let entry = CacheEntry::new(data, ttl);
		let serialized = serde_json::to_vec(&entry)?;
		let payload = self.encode_payload(&serialized)?;

		self
			.with_retry("set", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();
				let payload = payload.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let _: () = con.set_ex(&cache_key, payload, ttl).await?;
					Result::<_, CacheError>::Ok(())
				})
			})
			.await?;

		info!("set {} (TTL: {}s)", key, ttl);
		Ok(())
	}

	/// GET + TTL pipelined in one round-trip.
	///
	/// Returns `(value, age_seconds)` on hit, where age is derived from Redis:
	///   age = original_ttl − ttl_remaining
	///
	/// Redis is the sole clock. No SystemTime involved.
	#[instrument(skip(self), fields(key = %key))]
	pub async fn get<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>, CacheError> {
		let (value, _age) = self.get_with_age(key).await?;
		Ok(value)
	}

	/// Like `get`, but also returns the entry's age in seconds.
	/// Age is derived from Redis TTL — no stored timestamp.
	#[instrument(skip(self), fields(key = %key))]
	pub async fn get_with_age<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<(Option<T>, u64), CacheError> {
		let cache_key = self.make_key(key);
		let ns = namespace_of(key);

		let (raw, ttl_remaining): (Option<Vec<u8>>, i64) = self
			.with_retry("get", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					// Single round-trip: GET + TTL pipelined.
					let result: (Option<Vec<u8>>, i64) = redis::pipe().cmd("GET").arg(&cache_key).cmd("TTL").arg(&cache_key).query_async(&mut con).await?;
					Result::<_, CacheError>::Ok(result)
				})
			})
			.await?;

		match raw {
			None => {
				if let Ok(c) = &*CACHE_MISSES {
					c.with_label_values(&[ns]).inc();
				}
				Ok((None, 0))
			}
			Some(bytes) => {
				if let Ok(c) = &*CACHE_HITS {
					c.with_label_values(&[ns]).inc();
				}

				let decoded = self.decode_payload(&bytes)?;
				let entry: CacheEntry<T> = serde_json::from_slice(&decoded)?;

				// Age derived from Redis, not SystemTime.
				let age = if ttl_remaining > 0 {
					entry.ttl.saturating_sub(ttl_remaining as u64)
				} else {
					0 // key has no TTL or already expired (race)
				};

				if self.should_touch() {
					let _ = self.touch_entry(&cache_key, entry.ttl).await.map_err(|e| {
						warn!("touch failed for {}: {}", key, e);
						e
					});
				}

				info!("hit {} (age: {}s)", key, age);
				Ok((Some(entry.data), age))
			}
		}
	}

	/// Fetch and decode a cache entry's raw serialized bytes without
	/// deserializing into a concrete type.
	///
	/// Returns `None` on miss. On hit, returns the postcard-encoded payload
	/// (post flag-byte stripping and zstd decompression). The caller is
	/// responsible for size-checking and deserialization.
	///
	/// Used by `Store::fetch_capture` and `Store::read_artifact` to enforce
	/// `MAX_PAYLOAD_BYTES` before deserialization.
	pub async fn get_raw_payload(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
		let cache_key = self.make_key(key);

		let raw: Option<Vec<u8>> = self
			.with_retry("get_raw", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let result: Option<Vec<u8>> = con.get(&cache_key).await?;
					Result::<_, CacheError>::Ok(result)
				})
			})
			.await?;

		match raw {
			None => Ok(None),
			Some(bytes) => Ok(Some(self.decode_payload(&bytes)?)),
		}
	}

	/// Deserialize postcard bytes (from `get_raw_payload`) into `T`.
	///
	/// Thin wrapper so callers don't take a direct dependency on postcard.
	pub fn deserialize_payload<T: for<'de> serde::Deserialize<'de>>(&self, bytes: &[u8]) -> Result<T, CacheError> {
		serde_json::from_slice::<'_, T>(bytes).map_err(CacheError::SerializationError)
	}

	#[instrument(skip(self, data), fields(key = %key))]
	pub async fn set_binary(&self, key: &str, data: &[u8], content_type: Option<String>, ttl: Option<u64>) -> Result<(), CacheError> {
		let cache_key = self.make_key(key);
		let ttl = ttl.unwrap_or(self.config.default_ttl);

		let entry = BinaryCacheEntry::new(data.to_vec(), content_type, ttl);
		let serialized = serde_json::to_vec(&entry)?;
		let payload = self.encode_payload(&serialized)?;

		self
			.with_retry("set_binary", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();
				let payload = payload.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let _: () = con.set_ex(&cache_key, payload, ttl).await?;
					Result::<_, CacheError>::Ok(())
				})
			})
			.await?;

		info!("set_binary {} ({} bytes, TTL: {}s)", key, data.len(), ttl);
		Ok(())
	}

	#[instrument(skip(self), fields(key = %key))]
	pub async fn get_binary(&self, key: &str) -> Result<Option<(Vec<u8>, Option<String>)>, CacheError> {
		let cache_key = self.make_key(key);
		let ns = namespace_of(key);

		let (raw, ttl_remaining): (Option<Vec<u8>>, i64) = self
			.with_retry("get_binary", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let result: (Option<Vec<u8>>, i64) = redis::pipe().cmd("GET").arg(&cache_key).cmd("TTL").arg(&cache_key).query_async(&mut con).await?;
					Result::<_, CacheError>::Ok(result)
				})
			})
			.await?;

		match raw {
			None => {
				if let Ok(c) = &*CACHE_MISSES {
					c.with_label_values(&[ns]).inc();
				}
				Ok(None)
			}
			Some(bytes) => {
				if let Ok(c) = &*CACHE_HITS {
					c.with_label_values(&[ns]).inc();
				}

				let decoded = self.decode_payload(&bytes)?;
				let entry: BinaryCacheEntry = serde_json::from_slice(&decoded)?;

				let age = if ttl_remaining > 0 { entry.ttl.saturating_sub(ttl_remaining as u64) } else { 0 };

				if self.should_touch() {
					let _ = self.touch_entry(&cache_key, entry.ttl).await.map_err(|e| {
						warn!("touch failed for {}: {}", key, e);
						e
					});
				}

				info!("get_binary hit {} (age: {}s, {} bytes)", key, age, entry.data.len());
				Ok(Some((entry.data, entry.content_type)))
			}
		}
	}

	#[instrument(skip(self), fields(key = %key))]
	pub async fn delete(&self, key: &str) -> Result<bool, CacheError> {
		let cache_key = self.make_key(key);

		let deleted: i32 = self
			.with_retry("delete", || {
				let redis_client = self.redis_client.clone();
				let cache_key = cache_key.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let result: i32 = con.del(&cache_key).await?;
					Result::<_, CacheError>::Ok(result)
				})
			})
			.await?;

		Ok(deleted > 0)
	}

	#[instrument(skip(self))]
	pub async fn flush_all(&self) -> Result<u64, CacheError> {
		let pattern = format!("{}*", self.config.key_prefix);

		let deleted: u64 = self
			.with_retry("flush_all", || {
				let redis_client = self.redis_client.clone();
				let pattern = pattern.clone();

				Box::pin(async move {
					let mut con = redis_client.get_multiplexed_async_connection().await?;
					let keys: Vec<String> = con.keys(&pattern).await?;
					if keys.is_empty() {
						return Ok(0);
					}
					let deleted: i32 = con.del(&keys).await?;
					Result::<_, CacheError>::Ok(deleted as u64)
				})
			})
			.await?;

		info!("flushed {} entries", deleted);
		Ok(deleted)
	}
}

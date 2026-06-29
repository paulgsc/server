use moka::future::Cache;
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc, time::Instant};
use tracing::instrument;

use crate::{
	error::DedupCacheError,
	metrics::{namespace_of, DEDUP_WAITERS, FETCH_DURATION},
	store::CacheStore,
};

// ── DedupCache ────────────────────────────────────────────────────────────────
//
// Thundering-herd guard over CacheStore.
//
// This is where application-layer metrics get recorded, because this is where
// application-layer events actually happen:
//
//   FETCH_DURATION  — time from miss detection to Redis write. redis_exporter
//                     cannot see upstream fetch latency.
//
//   DEDUP_WAITERS   — requests that coalesced behind an in-flight fetch.
//                     redis_exporter cannot see this at all.
//
// Hit/miss counters are recorded in CacheStore (per namespace) because that's
// where the Redis GET happens. Everything else (connection latency, command
// throughput, memory) is delegated to redis_exporter.

pub struct DedupCache {
	store: Arc<CacheStore>,
	/// In-memory dedup layer. Stores bincode bytes (type-erased).
	/// moka::get_with guarantees the init closure runs once per key across
	/// concurrent callers; all others await the same future.
	in_flight: Cache<String, Arc<[u8]>>,
}

impl DedupCache {
	pub fn new(store: Arc<CacheStore>, max_in_flight: u64) -> Self {
		Self {
			store,
			in_flight: Cache::builder().max_capacity(max_in_flight).build(),
		}
	}

	// ── Generic ───────────────────────────────────────────────────────────

	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch<T, F, Fut>(&self, id: &str, fetcher: F) -> Result<(T, bool), DedupCacheError>
	where
		T: Serialize + for<'de> Deserialize<'de>,
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<T, DedupCacheError>> + Send,
	{
		self.get_or_fetch_with_ttl(id, None, fetcher).await
	}

	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch_with_ttl<T, F, Fut>(&self, id: &str, ttl: Option<u64>, fetcher: F) -> Result<(T, bool), DedupCacheError>
	where
		T: Serialize + for<'de> Deserialize<'de>,
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<T, DedupCacheError>> + Send,
	{
		// ① Redis hit — bypass moka entirely.
		if let Some(cached) = self.store.get::<T>(id).await? {
			return Ok((cached, true));
		}

		// ② Dedup gate.
		//    The closure only executes for the first caller; all racing callers
		//    await the same future and receive the same Arc<[u8]>.
		let ns = namespace_of(id).to_string();
		let key = id.to_string();
		let store = Arc::clone(&self.store);
		let mut is_fetcher = false;

		let bytes: Arc<[u8]> = self
			.in_flight
			.try_get_with(key.clone(), async {
				is_fetcher = true;
				let t = Instant::now();

				let value = fetcher().await.map_err(|e| e.to_string())?;
				store.set(&key, &value, ttl).await.map_err(|e| e.to_string())?;

				// Record fetch latency only for the fetcher, not waiters.
				if let Ok(h) = &*FETCH_DURATION {
					h.with_label_values(&[&ns]).observe(t.elapsed().as_secs_f64());
				}

				let bytes: Arc<[u8]> = serde_json::to_vec(&value).map_err(|e| e.to_string())?.into();

				Ok::<Arc<[u8]>, String>(bytes)
			})
			.await
			.map_err(|e| DedupCacheError::OperationError(e.to_string()))?;

		// If this caller was a waiter (not the fetcher), record it.
		if !is_fetcher {
			if let Ok(c) = &*DEDUP_WAITERS {
				c.with_label_values(&[&ns]).inc();
			}
		}

		let value: T = serde_json::from_slice(&bytes).map_err(DedupCacheError::SerializationError)?;

		// `is_fetcher`: false = returned from cache or dedup wait, true = we ran the fetch.
		// Return `cached = !is_fetcher` so callers have a consistent "was this a cache hit"
		// signal regardless of whether it came from Redis or the in-memory dedup layer.
		Ok((value, !is_fetcher))
	}

	// ── Binary ────────────────────────────────────────────────────────────

	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch_binary<F, Fut>(&self, id: &str, fetcher: F) -> Result<((Vec<u8>, Option<String>), bool), DedupCacheError>
	where
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<(Vec<u8>, Option<String>), DedupCacheError>> + Send,
	{
		self.get_or_fetch_binary_with_ttl(id, None, fetcher).await
	}

	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch_binary_with_ttl<F, Fut>(&self, id: &str, ttl: Option<u64>, fetcher: F) -> Result<((Vec<u8>, Option<String>), bool), DedupCacheError>
	where
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<(Vec<u8>, Option<String>), DedupCacheError>> + Send,
	{
		// ① Redis hit.
		if let Some(cached) = self.store.get_binary(id).await? {
			return Ok((cached, true));
		}

		// ② Dedup gate.
		let ns = namespace_of(id).to_string();
		let key = id.to_string();
		let store = Arc::clone(&self.store);
		let mut is_fetcher = false;

		let bytes: Arc<[u8]> = self
			.in_flight
			.try_get_with(key.clone(), async {
				is_fetcher = true;
				let t = Instant::now();

				let (data, content_type) = fetcher().await.map_err(|e| e.to_string())?;
				store.set_binary(&key, &data, content_type.clone(), ttl).await.map_err(|e| e.to_string())?;

				if let Ok(h) = &*FETCH_DURATION {
					h.with_label_values(&[&ns]).observe(t.elapsed().as_secs_f64());
				}

				let bytes: Arc<[u8]> = serde_json::to_vec(&(&data, &content_type)).map_err(|e| e.to_string())?.into();

				Ok::<Arc<[u8]>, String>(bytes)
			})
			.await
			.map_err(|e| DedupCacheError::OperationError(e.to_string()))?;

		if !is_fetcher {
			if let Ok(c) = &*DEDUP_WAITERS {
				c.with_label_values(&[&ns]).inc();
			}
		}

		let (data, content_type): (Vec<u8>, Option<String>) = serde_json::from_slice(&bytes).map_err(DedupCacheError::SerializationError)?;

		Ok(((data, content_type), !is_fetcher))
	}

	// ── Maintenance ───────────────────────────────────────────────────────

	#[instrument(skip(self))]
	pub async fn delete(&self, key: &str) -> Result<bool, DedupCacheError> {
		self.in_flight.remove(key).await;
		Ok(self.store.delete(key).await?)
	}

	#[instrument(skip(self))]
	pub async fn flush_all(&self) -> Result<u64, DedupCacheError> {
		self.in_flight.invalidate_all();
		self.in_flight.run_pending_tasks().await;
		Ok(self.store.flush_all().await?)
	}

	pub fn store(&self) -> Arc<CacheStore> {
		Arc::clone(&self.store)
	}
}

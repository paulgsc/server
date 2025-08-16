use super::{CacheError, CacheStore};
use crate::FileHostError;
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::{Arc, OnceLock};
use tracing::instrument;

/// Represents the different types of data that can be cached
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CacheData {
	/// Generic serializable data
	Generic(serde_json::Value),
	/// Binary data with optional content type
	Binary { data: Vec<u8>, content_type: Option<String> },
}

impl CacheData {
	/// Create a generic cache data entry from a serializable type
	pub fn from_generic<T: Serialize>(data: &T) -> Result<Self, FileHostError> {
		let value = serde_json::to_value(data)?;
		Ok(CacheData::Generic(value))
	}

	/// Create a binary cache data entry
	pub fn from_binary(data: Vec<u8>, content_type: Option<String>) -> Self {
		CacheData::Binary { data, content_type }
	}

	/// Extract generic data, converting it to the desired type
	pub fn into_generic<T: for<'de> Deserialize<'de>>(self) -> Result<T, FileHostError> {
		match self {
			CacheData::Generic(value) => {
				let result = serde_json::from_value(value)?;
				Ok(result)
			}
			CacheData::Binary { .. } => Err(CacheError::OperationError("Expected generic data, found binary data".to_string()))?,
		}
	}

	/// Extract binary data
	pub fn into_binary(self) -> Result<(Vec<u8>, Option<String>), FileHostError> {
		match self {
			CacheData::Binary { data, content_type } => Ok((data, content_type)),
			CacheData::Generic(_) => Err(CacheError::OperationError("Expected binary data, found generic data".to_string()))?,
		}
	}
}

/// A generic, bounded cache that can handle both generic serializable data and binary data.
///
/// It uses a `moka::future::Cache` to automatically manage the size of the
/// in-flight fetches, preventing unbounded growth. The cache stores a `OnceLock`
/// for each key, ensuring that a fetch for the same key is only executed once.
pub struct DedupCache {
	store: Arc<CacheStore>,
	active_fetches: Cache<String, Arc<OnceLock<CacheData>>>,
}

impl DedupCache {
	/// Creates a new `DedupCache`.
	///
	/// `max_in_flight`: The maximum number of concurrent fetches to keep in the
	/// `active_fetches` cache. Oldest entries will be evicted automatically.
	pub fn new(store: Arc<CacheStore>, max_in_flight: u64) -> Self {
		Self {
			store,
			active_fetches: Cache::builder().max_capacity(max_in_flight).build(),
		}
	}

	/// Retrieves generic data from the cache or fetches it if not found.
	///
	/// This method ensures that if multiple requests for the same key arrive concurrently,
	/// only one fetch operation is performed.
	/// Returns `(item, is_cached)`, where `is_cached` is `true` if the item
	/// was found in the cache, and `false` if it was fetched.
	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch<T, F, Fut>(&self, id: &str, fetcher: F) -> Result<(T, bool), FileHostError>
	where
		T: Clone + Send + Sync + 'static + Serialize + for<'de> Deserialize<'de>,
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<T, FileHostError>> + Send,
	{
		self.get_or_fetch_with_ttl(id, None, fetcher).await
	}

	/// Retrieves generic data from the cache or fetches it if not found, with custom TTL.
	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch_with_ttl<T, F, Fut>(&self, id: &str, ttl: Option<u64>, fetcher: F) -> Result<(T, bool), FileHostError>
	where
		T: Clone + Send + Sync + 'static + Serialize + for<'de> Deserialize<'de>,
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<T, FileHostError>> + Send,
	{
		// First, check the persistent store for generic data
		if let Some(cached_data) = self.store.get::<T>(id).await? {
			return Ok((cached_data, true));
		}

		// If not found, check the in-flight cache for an ongoing fetch
		let key = id.to_string();

		let once = self
			.active_fetches
			.try_get_with(key.clone(), async { Result::<Arc<OnceLock<CacheData>>, String>::Ok(Arc::new(OnceLock::new())) })
			.await
			.map_err(|e| CacheError::OperationError(format!("Failed to get or insert in active_fetches: {}", e)))?;

		// If another task is already fetching, wait for it to complete.
		if let Some(cache_data) = once.get() {
			let result = cache_data.clone().into_generic::<T>()?;
			return Ok((result, true));
		}

		// Otherwise, this is the first task to start the fetch.
		let fetch_result = fetcher().await?;

		// Cache the result in the store if the fetch was successful.
		self.store.set(&key, &fetch_result, ttl).await?;

		// Create cache data and signal other waiting tasks with the result.
		let cache_data = CacheData::from_generic(&fetch_result)?;
		let _ = once
			.set(cache_data)
			.map_err(|_| CacheError::OperationError("Failed to set result in OnceLock".to_string()))?;

		Ok((fetch_result, false))
	}

	/// Retrieves binary data from the cache or fetches it if not found.
	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch_binary<F, Fut>(&self, id: &str, fetcher: F) -> Result<((Vec<u8>, Option<String>), bool), FileHostError>
	where
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<(Vec<u8>, Option<String>), FileHostError>> + Send,
	{
		self.get_or_fetch_binary_with_ttl(id, None, fetcher).await
	}

	/// Retrieves binary data from the cache or fetches it if not found, with custom TTL.
	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch_binary_with_ttl<F, Fut>(&self, id: &str, ttl: Option<u64>, fetcher: F) -> Result<((Vec<u8>, Option<String>), bool), FileHostError>
	where
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<(Vec<u8>, Option<String>), FileHostError>> + Send,
	{
		// First, check the persistent store for binary data
		if let Some(cached_data) = self.store.get_binary(id).await? {
			return Ok((cached_data, true));
		}

		// If not found, check the in-flight cache for an ongoing fetch
		let key = id.to_string();

		let once = self
			.active_fetches
			.try_get_with(key.clone(), async { Result::<Arc<OnceLock<CacheData>>, String>::Ok(Arc::new(OnceLock::new())) })
			.await
			.map_err(|e| CacheError::OperationError(format!("Failed to get or insert in active_fetches: {}", e)))?;

		// If another task is already fetching, wait for it to complete.
		if let Some(cache_data) = once.get() {
			let result = cache_data.clone().into_binary()?;
			return Ok((result, true));
		}

		// Otherwise, this is the first task to start the fetch.
		let (data, content_type) = fetcher().await?;

		// Cache the result in the store if the fetch was successful.
		self.store.set_binary(&key, &data, content_type.clone(), ttl).await?;

		// Create cache data and signal other waiting tasks with the result.
		let cache_data = CacheData::from_binary(data.clone(), content_type.clone());
		let _ = once
			.set(cache_data)
			.map_err(|_| CacheError::OperationError("Failed to set result in OnceLock".to_string()))?;

		Ok(((data, content_type), false))
	}

	/// Unified method that can handle both generic and binary data based on the fetcher return type.
	/// This is useful when you want to cache different types of data with the same key namespace.
	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch_unified<F, Fut>(&self, id: &str, fetcher: F) -> Result<(CacheData, bool), FileHostError>
	where
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<CacheData, FileHostError>> + Send,
	{
		self.get_or_fetch_unified_with_ttl(id, None, fetcher).await
	}

	/// Unified method with custom TTL that can handle both generic and binary data.
	#[instrument(skip(self, fetcher))]
	pub async fn get_or_fetch_unified_with_ttl<F, Fut>(&self, id: &str, ttl: Option<u64>, fetcher: F) -> Result<(CacheData, bool), FileHostError>
	where
		F: FnOnce() -> Fut + Send,
		Fut: Future<Output = Result<CacheData, FileHostError>> + Send,
	{
		let key = id.to_string();

		// Check if we can find it in either cache type
		// Try generic first
		if let Ok(Some(generic_data)) = self.store.get::<serde_json::Value>(id).await {
			return Ok((CacheData::Generic(generic_data), true));
		}

		// Try binary
		if let Ok(Some((data, content_type))) = self.store.get_binary(id).await {
			return Ok((CacheData::from_binary(data, content_type), true));
		}

		let once = self
			.active_fetches
			.try_get_with(key.clone(), async { Result::<Arc<OnceLock<CacheData>>, String>::Ok(Arc::new(OnceLock::new())) })
			.await
			.map_err(|e| CacheError::OperationError(format!("Failed to get or insert in active_fetches: {}", e)))?;

		// If another task is already fetching, wait for it to complete.
		if let Some(cache_data) = once.get() {
			return Ok((cache_data.clone(), true));
		}

		// Otherwise, this is the first task to start the fetch.
		let fetch_result = fetcher().await?;

		// Cache the result in the appropriate store based on the data type
		match &fetch_result {
			CacheData::Generic(value) => {
				self.store.set(&key, value, ttl).await?;
			}
			CacheData::Binary { data, content_type } => {
				self.store.set_binary(&key, data, content_type.clone(), ttl).await?;
			}
		}

		// Signal other waiting tasks with the result.
		let _ = once
			.set(fetch_result.clone())
			.map_err(|_| CacheError::OperationError("Failed to set result in OnceLock".to_string()))?;

		Ok((fetch_result, false))
	}

	/// Delete a cache entry
	#[instrument(skip(self))]
	pub async fn delete(&self, key: &str) -> Result<bool, FileHostError> {
		// Remove from in-flight cache to prevent stale fetches
		self.active_fetches.remove(key).await;

		// Remove from persistent store
		self.store.delete(key).await
	}

	/// Flush all cache entries
	#[instrument(skip(self))]
	pub async fn flush_all(&self) -> Result<u64, FileHostError> {
		// Clear in-flight cache
		self.active_fetches.run_pending_tasks().await;
		self.active_fetches.invalidate_all();

		// Clear persistent store
		self.store.flush_all().await
	}
}

use crate::metrics::otel::{record_cache_hit, OperationTimer};
use crate::{AppState, FileHostError};
use serde::{Deserialize, Serialize};
use some_cache::DedupCacheError;
use std::future::Future;

/// Shared "cache-key → timer → dedup_cache.get_or_fetch → record_cache_hit"
/// shape used by every read handler that fronts an upstream fetch (Sheets or
/// Drive) with the dedup cache. This only removes that wrapping boilerplate —
/// callers still own the fetch closure, including any transform that needs
/// to happen inside it (so it's covered by the cache) or after the call
/// returns (so it re-runs on every request, cache hit or not).
pub async fn fetch_cached<T, F, Fut>(state: &AppState, operation: &'static str, cache_key: &str, fetch: F) -> Result<(T, bool), FileHostError>
where
	T: Serialize + for<'de> Deserialize<'de>,
	F: FnOnce() -> Fut + Send,
	Fut: Future<Output = Result<T, DedupCacheError>> + Send,
{
	let _timer = OperationTimer::new(operation, "total");
	let (data, was_cached) = state.realtime.dedup_cache.get_or_fetch(cache_key, fetch).await?;
	record_cache_hit(operation, was_cached);
	Ok((data, was_cached))
}

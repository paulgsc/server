use crate::{AppState, FileHostError};
use redis::AsyncCommands;

// Invariant: all Redis key construction funnels through these two functions.
pub fn session_cache_key(session_id: &str) -> String {
	format!("capture:session:{}", session_id)
}

pub const INDEX_KEY: &str = "capture:index";

// Epoch ms score for the sorted index, parsed from ISO 8601.
// Postcondition: returns 0.0 on parse failure rather than propagating.
pub fn epoch_ms_score(iso: &str) -> f64 {
	iso.parse::<chrono::DateTime<chrono::Utc>>().map(|dt| dt.timestamp_millis() as f64).unwrap_or(0.0)
}

// Index operations — raw Redis, not behind dedup cache.
// Invariant: only model-layer functions touch INDEX_KEY directly.
pub async fn index_insert(state: &AppState, session_id: &str, score: f64) -> Result<(), FileHostError> {
	let store = state.realtime.dedup_cache.store();
	let mut con = store
		.redis_client()
		.get_multiplexed_async_connection()
		.await
		.map_err(|e| FileHostError::OperationError(e.to_string()))?;
	let _: () = con.zadd(INDEX_KEY, session_id, score).await.map_err(|e| FileHostError::OperationError(e.to_string()))?;
	Ok(())
}

pub async fn index_remove(state: &AppState, session_id: &str) -> Result<(), FileHostError> {
	let store = state.realtime.dedup_cache.store();
	let mut con = store
		.redis_client()
		.get_multiplexed_async_connection()
		.await
		.map_err(|e| FileHostError::OperationError(e.to_string()))?;
	let _: () = con.zrem(INDEX_KEY, session_id).await.map_err(|e| FileHostError::OperationError(e.to_string()))?;
	Ok(())
}

pub async fn index_range(state: &AppState, from: f64, to: f64, limit: usize) -> Result<Vec<String>, FileHostError> {
	let store = state.realtime.dedup_cache.store();
	let mut con = store
		.redis_client()
		.get_multiplexed_async_connection()
		.await
		.map_err(|e| FileHostError::OperationError(e.to_string()))?;
	// ZRANGEBYSCORE ascending; caller reverses for newest-first.
	let ids: Vec<String> = con
		.zrangebyscore_limit(INDEX_KEY, from, to, 0, limit as isize)
		.await
		.map_err(|e| FileHostError::OperationError(e.to_string()))?;
	Ok(ids)
}

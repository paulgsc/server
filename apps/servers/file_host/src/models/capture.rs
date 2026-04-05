use crate::{AppState, DedupError};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Domain(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentKind(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedContent {
	pub kind: ContentKind,
	pub title: String,
	pub summary: String,
	pub headings: Vec<String>,
	pub keywords: Vec<String>,
	pub raw_length: u64,
	pub meta: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabCapture {
	pub tab_id: i64,
	pub url: String,
	pub tab_title: String,
	pub captured_at: String,
	pub extractor: String,
	pub domain: Domain,
	pub content: ExtractedContent,
	pub extraction_ok: bool,
	pub extraction_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedTab {
	pub tab_id: i64,
	pub url: String,
	pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureSession {
	pub session_id: String,
	pub captured_at: String,
	pub extension_version: String,
	pub total_open_tabs: u64,
	pub captures: Vec<TabCapture>,
	pub skipped: Vec<SkippedTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureSummary {
	pub session_id: String,
	pub captured_at: String,
	pub total_tabs: u64,
	pub captured_ok: usize,
	pub captured_fail: usize,
	pub skipped: usize,
}

impl From<&CaptureSession> for CaptureSummary {
	fn from(s: &CaptureSession) -> Self {
		Self {
			session_id: s.session_id.clone(),
			captured_at: s.captured_at.clone(),
			total_tabs: s.total_open_tabs,
			captured_ok: s.captures.iter().filter(|c| c.extraction_ok).count(),
			captured_fail: s.captures.iter().filter(|c| !c.extraction_ok).count(),
			skipped: s.skipped.len(),
		}
	}
}

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
pub async fn index_insert(state: &AppState, session_id: &str, score: f64) -> Result<(), DedupError> {
	let store = state.realtime.dedup_cache.get_cache_store();
	let mut con = store
		.redis_client()
		.get_multiplexed_async_connection()
		.await
		.map_err(|e| DedupError::OperationError(e.to_string()))?;
	let _: () = con.zadd(INDEX_KEY, session_id, score).await.map_err(|e| DedupError::OperationError(e.to_string()))?;
	Ok(())
}

pub async fn index_remove(state: &AppState, session_id: &str) -> Result<(), DedupError> {
	let store = state.realtime.dedup_cache.get_cache_store();
	let mut con = store
		.redis_client()
		.get_multiplexed_async_connection()
		.await
		.map_err(|e| DedupError::OperationError(e.to_string()))?;
	let _: () = con.zrem(INDEX_KEY, session_id).await.map_err(|e| DedupError::OperationError(e.to_string()))?;
	Ok(())
}

pub async fn index_range(state: &AppState, from: f64, to: f64, limit: usize) -> Result<Vec<String>, DedupError> {
	let store = state.realtime.dedup_cache.get_cache_store();
	let mut con = store
		.redis_client()
		.get_multiplexed_async_connection()
		.await
		.map_err(|e| DedupError::OperationError(e.to_string()))?;
	// ZRANGEBYSCORE ascending; caller reverses for newest-first.
	let ids: Vec<String> = con
		.zrangebyscore_limit(INDEX_KEY, from, to, 0, limit as isize)
		.await
		.map_err(|e| DedupError::OperationError(e.to_string()))?;
	Ok(ids)
}

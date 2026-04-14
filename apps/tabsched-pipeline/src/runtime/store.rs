///
/// This is the single point of all IO for the pipeline daemon.
/// No std::fs calls exist anywhere else in the crate.
///
/// Contract:
///   - All reads/writes are namespaced under known key prefixes (see job.rs).
///   - All writes carry an explicit TTL — no indefinite retention.
///   - Serialization is postcard (binary) via some-cache CacheStore.
///   - The capture session is READ from the key written by the Axum capture
///     handler. The pipeline never writes to capture:session:* keys.
///
/// Wire format note:
///   If the Axum capture handler writes CaptureSession as raw JSON (pre-migration),
///   fetch_capture will fail to deserialize with a postcard error. In that case,
///   use fetch_capture_json (feature-flagged below) until the handler is migrated.
use anyhow::{Context, Result};
use redis::AsyncCommands;
use serde::{de::DeserializeOwned, Serialize};
use tracing::instrument;

use some_cache::{CacheConfig, CacheEntry, CacheStore, StreamHandle};

use super::job::{artifact_key, capture_key, state_key, JobRecord, ARTIFACT_TTL_SECS};
use ws_events::tabsched::CaptureSession;

/// Reject any single payload larger than this before deserialisation.
/// Acts as a poison guard at ingestion — prevents OOM on malformed entries.
/// 8 MB is well above any realistic capture session or pipeline artifact.
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;

// ── Store ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Store {
	/// Typed cache for all job state and pipeline artifacts.
	/// Handles serialization (postcard), compression (zstd), retry, and TTL.
	cache: CacheStore,

	/// Raw Redis connection for operations with no CacheStore equivalent:
	/// currently only push_dlq (LPUSH + LTRIM).
	///
	/// ConnectionManager provides auto-reconnect without per-call connection
	/// overhead — appropriate for the low-frequency DLQ path.
	dlq_conn: redis::aio::ConnectionManager,
}

impl Store {
	pub async fn connect(redis_url: &str) -> Result<Self> {
		// ── CacheStore ────────────────────────────────────────────────────
		//
		// key_prefix is empty: key functions in job.rs produce fully-qualified
		// keys (e.g. "pipeline:state:{id}") and own the namespace contract.
		// Delegating prefix construction to CacheStore would split that
		// responsibility without benefit.
		//
		// Compression is enabled with a conservative threshold: pipeline
		// artifacts (embed vectors, edge graphs) are the hot path and will
		// compress well above 1 KiB. Job state records are typically small
		// and will fall below the threshold — no CPU cost for them.
		let config = CacheConfig {
			redis_url: redis_url.to_string(),
			key_prefix: String::new(),
			default_ttl: ARTIFACT_TTL_SECS,
			max_retries: 3,
			retry_delay_ms: 50,
			enable_compression: true,
			compression_threshold: 1024,
			zstd_level: Some(3),
			touch_probability: Some(0.0), // pipeline artifacts are write-once; no sliding TTL
		};

		let cache = CacheStore::new(config).context("building CacheStore")?;

		// ── DLQ connection ────────────────────────────────────────────────
		let client = redis::Client::open(redis_url).context("invalid Redis URL")?;
		let dlq_conn = redis::aio::ConnectionManager::new(client).await.context("connecting to Redis (DLQ)")?;

		Ok(Self { cache, dlq_conn })
	}

	// ── Job state ─────────────────────────────────────────────────────────

	#[instrument(skip(self), fields(session_id = %record.session_id))]
	pub async fn write_state(&self, record: &JobRecord) -> Result<()> {
		let key = state_key(&record.session_id);
		self.cache.set(&key, record, Some(ARTIFACT_TTL_SECS)).await.context("write_state")
	}

	pub async fn read_state(&self, session_id: &str) -> Result<Option<JobRecord>> {
		let key = state_key(session_id);
		self.cache.get(&key).await.context("read_state")
	}

	// ── Capture session (read-only — written by Axum handler) ────────────

	/// Fetch the CaptureSession written by the Axum /capture endpoint.
	///
	/// Size guard runs on the raw encoded bytes before deserialization to
	/// prevent OOM on oversized or malformed payloads (poison detection).
	///
	/// Assumes the Axum handler writes CaptureSession via some-cache
	/// (postcard-encoded). If the handler is still writing raw JSON, use
	/// fetch_capture_json below and migrate together.
	#[instrument(skip(self), fields(session_id))]
	pub async fn fetch_capture(&self, session_id: &str) -> Result<CaptureSession> {
		let key = capture_key(session_id);

		// Use get_raw so we can enforce MAX_PAYLOAD_BYTES before deserializing.
		// CacheStore::get would deserialize eagerly with no size visibility.
		let raw = self
			.cache
			.get_raw_payload(&key)
			.await
			.context("fetch_capture get")?
			.ok_or_else(|| anyhow::anyhow!("capture session not found: {}", key))?;

		if raw.len() > MAX_PAYLOAD_BYTES {
			anyhow::bail!("capture payload too large: {} bytes (limit {})", raw.len(), MAX_PAYLOAD_BYTES);
		}

		let entry: CacheEntry<CaptureSession> = self.cache.deserialize_payload(&raw).map_err(|e| {
			// This preserves the structured CacheError inside the anyhow context
			// Convert bytes to string, replacing invalid UTF-8 with
			// Log the raw data along with the error
			tracing::error!(
				error = ?e,
				"failed to deserialize CacheEntry envelope"
			);
			anyhow::Error::from(e).context("failed to deserialize CacheEntry")
		})?;

		Ok(entry.data)
	}

	// ── Intermediate artifacts ────────────────────────────────────────────

	pub async fn write_artifact<T: Serialize>(&self, session_id: &str, stage: &str, value: &T) -> Result<()> {
		let key = artifact_key(session_id, stage);
		self
			.cache
			.set(&key, value, Some(ARTIFACT_TTL_SECS))
			.await
			.with_context(|| format!("write_artifact {}", stage))
	}

	pub async fn read_artifact<T: DeserializeOwned>(&self, session_id: &str, stage: &str) -> Result<Option<T>> {
		let key = artifact_key(session_id, stage);

		// Size guard: fetch raw bytes first, check size, then deserialize.
		// Mirrors the original guard; prevents OOM on artifact corruption.
		let raw = self.cache.get_raw_payload(&key).await.with_context(|| format!("read_artifact {}", stage))?;

		match raw {
			None => Ok(None),
			Some(bytes) => {
				if bytes.len() > MAX_PAYLOAD_BYTES {
					anyhow::bail!("artifact {} too large: {} bytes (limit {})", stage, bytes.len(), MAX_PAYLOAD_BYTES);
				}
				let value = self.cache.deserialize_payload::<T>(&bytes).with_context(|| format!("deserialize artifact {}", stage))?;
				Ok(Some(value))
			}
		}
	}

	/// Fetch the previous run's output topology.
	/// Missing key is not an error — this is the first run.
	pub async fn fetch_current_tracks(&self, session_id: &str) -> Result<Option<serde_json::Value>> {
		self.read_artifact(session_id, "current_tracks").await
	}

	// ── DLQ ──────────────────────────────────────────────────────────────

	/// Push a failed session onto the DLQ list for manual inspection.
	///
	/// Uses raw Redis (LPUSH + LTRIM) — CacheStore has no list primitive
	/// and the DLQ is not a cache entry. The DLQ connection is separate
	/// from the CacheStore connection path.
	///
	/// Capped at 1000 entries; oldest are trimmed on each push.
	pub async fn push_dlq(&mut self, session_id: &str, reason: &str) -> Result<()> {
		let entry = serde_json::json!({
				"session_id": session_id,
				"reason":     reason,
		});
		self.dlq_conn.lpush::<_, _, ()>("pipeline:dlq", entry.to_string()).await.context("push_dlq")?;
		self.dlq_conn.ltrim::<_, ()>("pipeline:dlq", 0, 999).await.context("dlq trim")?;
		Ok(())
	}

	/// Notify downstream consumers (like the CLI) that a pipeline run is finished.
	/// This is fire-and-forget; failures are logged but not returned as Errors
	/// to avoid stalling the pipeline.
	pub async fn notify_completion(&self, session_id: &str) {
		let stream = StreamHandle::pipeline_completed(self.cache.clone(), "pipeline-worker");
		if let Err(e) = stream.publish_completed(session_id).await {
			tracing::warn!(session_id, error = %e, "failed to publish completion event to stream");
		}
	}
}

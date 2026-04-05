///
/// This is the single point of all IO for the pipeline daemon.
/// No std::fs calls exist anywhere else in the crate.
///
/// Contract:
///   - All reads/writes are namespaced under known key prefixes (see job.rs).
///   - All writes carry an explicit TTL — no indefinite retention.
///   - Large blobs (CandidateGraph, PipelineOutput) are stored as
///     serde_json bytes; callers never see raw bytes.
///   - The capture session (source of truth) is READ from the key
///     written by the Axum capture handler.  The pipeline never writes
///     to capture:session:* keys.
use anyhow::{Context, Result};
use redis::AsyncCommands;
use serde::{de::DeserializeOwned, Serialize};
use tracing::instrument;

use super::job::{artifact_key, capture_key, state_key, JobRecord, ARTIFACT_TTL_SECS};
use crate::types::CaptureSession;

/// Size limit for any single payload fetched from Redis.
/// Payloads larger than this are rejected as poison at ingestion.
/// 8 MB is well above any realistic capture session.
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct Store {
	conn: redis::aio::ConnectionManager,
}

impl Store {
	pub async fn connect(url: &str) -> Result<Self> {
		let client = redis::Client::open(url).context("invalid Redis URL")?;
		let conn = redis::aio::ConnectionManager::new(client).await.context("connecting to Redis")?;
		Ok(Self { conn })
	}

	// ── Job state ─────────────────────────────────────────────────────────

	#[instrument(skip(self), fields(session_id = %record.session_id))]
	pub async fn write_state(&mut self, record: &JobRecord) -> Result<()> {
		let key = state_key(&record.session_id);
		let val = serde_json::to_string(record)?;
		self.conn.set_ex::<_, _, ()>(&key, val, ARTIFACT_TTL_SECS).await.context("write_state")?;
		Ok(())
	}

	pub async fn read_state(&mut self, session_id: &str) -> Result<Option<JobRecord>> {
		let key = state_key(session_id);
		let raw: Option<String> = self.conn.get(&key).await.context("read_state")?;
		raw.map(|s| serde_json::from_str(&s).context("deserialize JobRecord")).transpose()
	}

	// ── Capture session (read-only — written by Axum handler) ────────────

	/// Fetch the CaptureSession written by the Axum /capture endpoint.
	///
	/// Enforces MAX_PAYLOAD_BYTES before deserialisation.
	/// Returns StageError::Poison if the key is missing or oversized.
	#[instrument(skip(self), fields(session_id))]
	pub async fn fetch_capture(&mut self, session_id: &str) -> Result<CaptureSession> {
		let key = capture_key(session_id);
		let raw: Option<Vec<u8>> = self.conn.get(&key).await.context("fetch_capture get")?;

		let raw = raw.ok_or_else(|| anyhow::anyhow!("capture session not found in Redis: {}", session_id))?;

		if raw.len() > MAX_PAYLOAD_BYTES {
			anyhow::bail!("capture payload too large: {} bytes (limit {})", raw.len(), MAX_PAYLOAD_BYTES);
		}

		serde_json::from_slice(&raw).context("deserialize CaptureSession")
	}

	// ── Intermediate artifacts ────────────────────────────────────────────

	pub async fn write_artifact<T: Serialize>(&mut self, session_id: &str, stage: &str, value: &T) -> Result<()> {
		let key = artifact_key(session_id, stage);
		let val = serde_json::to_vec(value)?;
		self
			.conn
			.set_ex::<_, _, ()>(&key, val, ARTIFACT_TTL_SECS)
			.await
			.with_context(|| format!("write_artifact {}", stage))?;
		Ok(())
	}

	pub async fn read_artifact<T: DeserializeOwned>(&mut self, session_id: &str, stage: &str) -> Result<Option<T>> {
		let key = artifact_key(session_id, stage);
		let raw: Option<Vec<u8>> = self.conn.get(&key).await.with_context(|| format!("read_artifact {}", stage))?;
		raw
			.map(|b| {
				if b.len() > MAX_PAYLOAD_BYTES {
					anyhow::bail!("artifact {} too large: {} bytes", stage, b.len())
				}
				serde_json::from_slice(&b).with_context(|| format!("deserialize artifact {}", stage))
			})
			.transpose()
	}

	/// Fetch the current topology (PipelineOutput of the previous run) if it
	/// exists.  Used as a soft constraint in the track-grouping prompt.
	/// Missing key is not an error — first run.
	pub async fn fetch_current_tracks(&mut self, session_id: &str) -> Result<Option<serde_json::Value>> {
		self.read_artifact(session_id, "current_tracks").await
	}

	// ── DLQ ──────────────────────────────────────────────────────────────

	/// Push a session_id onto the DLQ list for manual inspection.
	/// Capped at 1000 entries — oldest are trimmed.
	pub async fn push_dlq(&mut self, session_id: &str, reason: &str) -> Result<()> {
		let entry = serde_json::json!({
				"session_id": session_id,
				"reason": reason,
				"at": chrono::Utc::now().to_rfc3339(),
		});
		self.conn.lpush::<_, _, ()>("pipeline:dlq", entry.to_string()).await.context("push_dlq")?;
		self.conn.ltrim::<_, ()>("pipeline:dlq", 0, 999).await.context("dlq trim")?;
		Ok(())
	}
}

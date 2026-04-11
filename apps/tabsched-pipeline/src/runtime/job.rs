///
/// A PipelineJob is the unit of work that moves through the daemon.
/// It arrives via JetStream, carries its session_id as an idempotency key,
/// and never carries the full capture payload — only the pointer needed
/// to fetch it from Redis.
///
/// State transitions (persisted to Redis on every change):
///
///   Pending → Embedding → Edges → Tracks → Completed
///                 ↓          ↓       ↓
///               Failed    Failed  Failed
///                 ↓
///              (DLQ publish)
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// All observable states a job can be in.
/// Persisted as `pipeline:state:<session_id>` in Redis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
	Pending,
	Embedding,
	Edges,
	Tracks,
	Completed,
	Failed { reason: String },
}

impl std::fmt::Display for JobState {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Pending => write!(f, "pending"),
			Self::Embedding => write!(f, "embedding"),
			Self::Edges => write!(f, "edges"),
			Self::Tracks => write!(f, "tracks"),
			Self::Completed => write!(f, "completed"),
			Self::Failed { reason } => write!(f, "failed: {}", reason),
		}
	}
}

/// Full state record stored in Redis alongside the job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
	pub session_id: String,
	pub state: JobState,
	pub created_at: DateTime<Utc>,
	pub updated_at: DateTime<Utc>,
	/// How many times this job has been attempted.
	pub attempts: u32,
}

impl JobRecord {
	pub fn new(session_id: &str) -> Self {
		let now = Utc::now();
		Self {
			session_id: session_id.to_string(),
			state: JobState::Pending,
			created_at: now,
			updated_at: now,
			attempts: 0,
		}
	}

	pub fn transition(&mut self, next: JobState) {
		tracing::info!(
				session_id = %self.session_id,
				from = %self.state,
				to   = %next,
				"job state transition"
		);
		self.state = next;
		self.updated_at = Utc::now();
	}
}

// ── Redis key helpers — all key construction funnels through here ─────────

pub fn state_key(session_id: &str) -> String {
	format!("pipeline:state:{}", session_id)
}

pub fn capture_key(session_id: &str) -> String {
	// Mirrors the Axum handler's `session_cache_key`.
	format!("cache:capture:session:{}", session_id)
}

pub fn artifact_key(session_id: &str, stage: &str) -> String {
	format!("pipeline:artifact:{}:{}", session_id, stage)
}

/// TTL applied to all pipeline artifacts in Redis.
/// After this window the job is assumed stale and keys are evicted.
/// 24 hours is intentionally generous — a pipeline run takes minutes.
pub const ARTIFACT_TTL_SECS: u64 = 86_400;

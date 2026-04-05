///
/// Every stage function returns StageError, not anyhow::Error.
/// The daemon's retry logic dispatches on the variant — Retryable gets
/// exponential backoff, Permanent goes straight to DLQ, Poison is
/// dropped with a warning (payload itself is broken).
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StageError {
	/// Transient failure — retry with backoff.
	/// Network timeouts, 503s, Redis connection drops, LLM overload.
	#[error("retryable: {0}")]
	Retryable(#[source] anyhow::Error),

	/// Permanent failure — do not retry, publish to DLQ subject.
	/// Schema validation failed, LLM returned unparseable JSON after
	/// exhausting retries, embed dimension mismatch.
	#[error("permanent: {0}")]
	Permanent(#[source] anyhow::Error),

	/// Poison payload — reject and discard, never retry.
	/// Payload exceeds size limit, session_id empty/invalid,
	/// zero valid captures after filter.
	#[error("poison: {0}")]
	Poison(#[source] anyhow::Error),
}

impl StageError {
	pub fn retryable(e: impl Into<anyhow::Error>) -> Self {
		Self::Retryable(e.into())
	}
	pub fn permanent(e: impl Into<anyhow::Error>) -> Self {
		Self::Permanent(e.into())
	}
	pub fn poison(e: impl Into<anyhow::Error>) -> Self {
		Self::Poison(e.into())
	}

	/// True if the error variant warrants a retry attempt.
	pub fn is_retryable(&self) -> bool {
		matches!(self, Self::Retryable(_))
	}
}

/// Retry policy parameters for a single stage.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
	pub max_attempts: u32,
	pub base_delay_ms: u64,
	pub max_delay_ms: u64,
}

impl RetryPolicy {
	pub const EMBED: Self = Self {
		max_attempts: 5,
		base_delay_ms: 2_000,
		max_delay_ms: 30_000,
	};

	pub const LLM: Self = Self {
		max_attempts: 3,
		base_delay_ms: 5_000,
		max_delay_ms: 60_000,
	};

	/// Exponential backoff delay for attempt `n` (0-indexed).
	pub fn delay_ms(&self, n: u32) -> u64 {
		let exp = self.base_delay_ms.saturating_mul(1u64 << n.min(10));
		exp.min(self.max_delay_ms)
	}
}

/// Run `f` with retry semantics derived from `policy`.
/// Only `StageError::Retryable` triggers a retry; other variants
/// short-circuit immediately.
pub async fn with_retry<F, Fut, T>(policy: RetryPolicy, mut f: F) -> Result<T, StageError>
where
	F: FnMut() -> Fut,
	Fut: std::future::Future<Output = Result<T, StageError>>,
{
	let mut attempt = 0u32;
	loop {
		match f().await {
			Ok(v) => return Ok(v),
			Err(StageError::Retryable(e)) if attempt + 1 < policy.max_attempts => {
				let delay = policy.delay_ms(attempt);
				tracing::warn!(
						attempt = attempt + 1,
						max = policy.max_attempts,
						delay_ms = delay,
						error = %e,
						"stage failed, retrying"
				);
				tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
				attempt += 1;
			}
			Err(StageError::Retryable(e)) => {
				// Exhausted retries — demote to Permanent.
				return Err(StageError::Permanent(anyhow::anyhow!("exhausted {} attempts: {}", policy.max_attempts, e)));
			}
			Err(e) => return Err(e),
		}
	}
}

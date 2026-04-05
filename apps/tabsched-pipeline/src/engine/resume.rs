// ── Stage resume helpers ──────────────────────────────────────────────────

/// Check Redis for an existing artifact; return it if present.
/// This makes each stage idempotent on redeliver: if the worker crashed
/// after writing "embed" but before writing "edges", a retry skips embed.

#[macro_export]
macro_rules! resume_or_run {
    ($store:expr, $session_id:expr, $stage:literal, $T:ty, $run:expr) => {{
        if let Some(cached) = $store
            .read_artifact::<$T>($session_id, $stage)
            .await
            .unwrap_or(None)
        {
            tracing::info!(
                session_id = %$session_id,
                stage = $stage,
                "resuming from cached artifact"
            );
            cached
        } else {
            let result = $run.await?;
            $store
                .write_artifact($session_id, $stage, &result)
                .await
                .map_err(crate::error::StageError::retryable)?;
            result
        }
    }};
}

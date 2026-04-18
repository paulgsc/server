use crate::{
	error::{with_retry, RetryPolicy, StageError},
	runtime::{JobRecord, JobState},
	stages::{derive_edges::run_derive_edges, derive_tracks::run_derive_tracks, embed::run_embed_stage, toml_gen::generate_toml},
	types::PipelineOutput,
};

use crate::resume_or_run;
use crate::runtime::WorkerCtx;
use tokio_util::sync::CancellationToken;
use tracing::{info, instrument};
use uuid::Uuid;

// ── Job processor ─────────────────────────────────────────────────────────

/// Process a single pipeline job end-to-end.
///
/// Returns Ok(()) on success (caller ACKs).
/// Returns Err(StageError) on failure (caller dispatches NAK/TERM).
///
/// Invariant: every intermediate artifact is written to Redis before the
/// next stage begins.  The function is therefore safe to retry from any
/// point — resume_or_run! skips already-completed stages.
#[instrument(skip(ctx, token), fields(session_id = %session_id))]
pub async fn process_job(ctx: &WorkerCtx, session_id: &str, token: CancellationToken) -> Result<(), StageError> {
	tokio::select! {
		biased;
		_ = token.cancelled() => {
			info!(session_id, "job preempted by shutdown — NAK for redeliver");
			Err(StageError::retryable(anyhow::anyhow!("shutdown mid-job")))
		}
		res = do_process_job(ctx, session_id) => res,
	}
}

/// Inner function — all existing process_job body moves here verbatim.
async fn do_process_job(ctx: &WorkerCtx, session_id: &str) -> Result<(), StageError> {
	// ── Fetch + validate CaptureSession from Redis ────────────────────────
	let session = ctx.store.clone().fetch_capture(session_id).await.map_err(|e| {
		// Missing key → Poison (extension never posted it, or TTL expired).
		// Oversized → Poison (guardrail in store.rs).
		StageError::poison(e)
	})?;

	// Validate session_id matches envelope (prevent key collisions).
	if session.session_id != session_id {
		return Err(StageError::poison(anyhow::anyhow!(
			"session_id mismatch: envelope={} payload={}",
			session_id,
			session.session_id
		)));
	}

	// Filter to valid captures.
	let captures: Vec<_> = session.captures.into_iter().filter(|c| c.extraction_ok).collect();

	if captures.is_empty() {
		return Err(StageError::poison(anyhow::anyhow!("no extraction_ok captures in session")));
	}

	// ── Update job state ──────────────────────────────────────────────────
	let mut record = ctx.store.clone().read_state(session_id).await.unwrap_or(None).unwrap_or_else(|| JobRecord::new(session_id));
	record.attempts += 1;

	// ── Stage 2: embed ────────────────────────────────────────────────────
	record.transition(JobState::Embedding);
	ctx.store.clone().write_state(&record).await.ok();

	let candidate_graph = resume_or_run!(
		ctx.store.clone(),
		session_id,
		"embed",
		crate::types::CandidateGraph,
		with_retry(RetryPolicy::EMBED, || {
			run_embed_stage(&ctx.http, &ctx.embed_provider, &captures, ctx.similarity_threshold)
		})
	);

	// ── Stage 3: derive edges ─────────────────────────────────────────────
	record.transition(JobState::Edges);
	ctx.store.clone().write_state(&record).await.ok();

	let (edges, rejected) = resume_or_run!(
		ctx.store.clone(),
		session_id,
		"edges",
		(Vec<crate::types::DerivedEdge>, Vec<crate::types::RejectedCandidate>),
		with_retry(RetryPolicy::LLM, || { run_derive_edges(ctx.llm.as_ref(), &candidate_graph, &ctx.edge_template) })
	);

	// ── Stage 4: derive tracks ────────────────────────────────────────────
	record.transition(JobState::Tracks);
	ctx.store.clone().write_state(&record).await.ok();

	// Fetch previous run's output as soft constraint (missing key = None = first run).
	let current_tracks = ctx.store.clone().fetch_current_tracks(session_id).await.unwrap_or(None);

	let (tracks, changes) = resume_or_run!(
		ctx.store.clone(),
		session_id,
		"tracks",
		(Vec<crate::types::DerivedTrack>, Vec<crate::types::TopologyChange>),
		with_retry(RetryPolicy::LLM, || {
			run_derive_tracks(ctx.llm.as_ref(), &candidate_graph, &edges, current_tracks.as_ref(), ctx.window_size, &ctx.track_template)
		})
	);

	// ── Assemble + persist PipelineOutput ────────────────────────────────
	let output = PipelineOutput {
		run_id: Uuid::new_v4().to_string(),
		run_at: chrono::Utc::now().to_rfc3339(),
		model: ctx.llm.model_name().to_string(),
		embed_model: ctx.embed_provider.model_name().to_string(),
		window_size: ctx.window_size,
		resources: candidate_graph.resources.clone(),
		edges,
		tracks,
		changes_from_current: changes,
		rejected_candidates: rejected,
	};

	ctx.store.clone().write_artifact(session_id, "output", &output).await.map_err(StageError::retryable)?;

	// Also write the TOML so downstream can read it from Redis by key
	// `pipeline:artifact:<session_id>:toml`.
	let toml_str = generate_toml(&output);
	ctx.store.clone().write_artifact(session_id, "toml", &toml_str).await.map_err(StageError::retryable)?;

	// ── Notify CLI consumers via Redis Stream ────────────────────────────
	// Non-fatal: failures won't block ACK.
	ctx.store.notify_completion(session_id).await;

	record.transition(JobState::Completed);
	ctx.store.clone().write_state(&record).await.ok();

	info!(resources = output.resources.len(), edges = output.edges.len(), tracks = output.tracks.len(), "job complete");

	Ok(())
}

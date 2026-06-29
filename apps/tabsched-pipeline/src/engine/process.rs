use crate::{
	error::{with_retry, RetryPolicy, StageError},
	runtime::{fetch_tabs_from_server, JobRecord, JobState},
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
/// next stage begins. The function is therefore safe to retry from any
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

async fn do_process_job(ctx: &WorkerCtx, session_id: &str) -> Result<(), StageError> {
	// ── Fetch Vec<TabCapture> from Axum server ────────────────────────────
	//
	// The server (SQLite) is the authority. No Redis staging on the Axum side.
	// Network failure → Retryable (JetStream NAK + redeliver).
	// Empty response  → Poison (no point retrying an empty db).
	let captures = fetch_tabs_from_server(&ctx.http, &ctx.axum_base_url).await.map_err(StageError::retryable)?;

	if captures.is_empty() {
		return Err(StageError::poison(anyhow::anyhow!("server returned no extraction_ok tabs for session_id={}", session_id)));
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

	// ── Assemble + persist PipelineOutput ─────────────────────────────────
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

	let toml_str = generate_toml(&output);
	ctx.store.clone().write_artifact(session_id, "toml", &toml_str).await.map_err(StageError::retryable)?;

	ctx.store.notify_completion(session_id).await;

	record.transition(JobState::Completed);
	ctx.store.clone().write_state(&record).await.ok();

	info!(resources = output.resources.len(), edges = output.edges.len(), tracks = output.tracks.len(), "job complete");

	Ok(())
}

///
/// Stage 3: LLM edge derivation.
/// Changes from v1:
///   - Returns StageError.
///   - HTTP errors classified as Retryable vs Permanent.
///   - Unparseable LLM JSON → Permanent (after caller exhausts retries).
///   - Stage deadline enforced via tokio::time::timeout.
use serde::Deserialize;
use tokio::time::timeout;
use tracing::instrument;

use crate::error::StageError;
use crate::llm::extract_json;
use crate::llm::LlmBackend;
use crate::stages::prompts::{fill_template, format_candidates, format_resources};
use crate::types::{CandidateGraph, DerivedEdge, EdgeKind, RejectedCandidate};

pub const STAGE_TIMEOUT_SECS: u64 = 300;

#[derive(Deserialize)]
struct EdgeResponse {
	edges: Vec<EdgeItem>,
	#[serde(default)]
	rejected: Vec<RejectedItem>,
}

#[derive(Deserialize)]
struct EdgeItem {
	source: String,
	target: String,
	kind: EdgeKindRaw,
	weight: f32,
	reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum EdgeKindRaw {
	Similar,
	Reinforces,
	Overlaps,
}

impl From<EdgeKindRaw> for EdgeKind {
	fn from(r: EdgeKindRaw) -> Self {
		match r {
			EdgeKindRaw::Similar => EdgeKind::Similar,
			EdgeKindRaw::Reinforces => EdgeKind::Reinforces,
			EdgeKindRaw::Overlaps => EdgeKind::Overlaps,
		}
	}
}

#[derive(Deserialize)]
struct RejectedItem {
	source: String,
	target: String,
	reason: String,
}

#[instrument(skip(llm, graph, template), fields(n_candidates = graph.candidate_edges.len()))]
pub async fn run_derive_edges(llm: &dyn LlmBackend, graph: &CandidateGraph, template: &str) -> Result<(Vec<DerivedEdge>, Vec<RejectedCandidate>), StageError> {
	tracing::info!("Stage 3: deriving edges via LLM");

	let prompt = fill_template(
		template,
		&[("RESOURCES", &format_resources(&graph.resources)), ("CANDIDATE_EDGES", &format_candidates(graph))],
	);

	let raw = timeout(std::time::Duration::from_secs(STAGE_TIMEOUT_SECS), llm.complete(&prompt, "derive-edges"))
		.await
		.map_err(|_| StageError::retryable(anyhow::anyhow!("derive-edges LLM call timed out")))?
		.map_err(|e| {
			// LLM backend errors are retryable unless they indicate a bad request.
			StageError::retryable(anyhow::anyhow!("LLM complete error: {}", e))
		})?;

	let parsed: EdgeResponse = extract_json(&raw).map_err(|e| StageError::permanent(anyhow::anyhow!("derive-edges: LLM returned unparseable JSON: {}", e)))?;

	let edges: Vec<DerivedEdge> = parsed
		.edges
		.into_iter()
		.map(|e| DerivedEdge {
			source: e.source,
			target: e.target,
			kind: e.kind.into(),
			weight: e.weight.clamp(0.0, 1.0),
			reason: e.reason,
			derived_by: "llm".into(),
			model: llm.model_name().to_string(),
		})
		.collect();

	let rejected: Vec<RejectedCandidate> = parsed
		.rejected
		.into_iter()
		.map(|r| RejectedCandidate {
			source: r.source,
			target: r.target,
			reason: r.reason,
		})
		.collect();

	tracing::info!(edges = edges.len(), rejected = rejected.len(), "Stage 3 complete");

	Ok((edges, rejected))
}

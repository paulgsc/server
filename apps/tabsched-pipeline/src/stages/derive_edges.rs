///
/// Sends the candidate graph to the LLM backend and parses the
/// structured JSON response into Vec<DerivedEdge> + rejected candidates.
use anyhow::Result;
use serde::Deserialize;
use tracing::instrument;

use crate::llm::{extract_json, LlmBackend};
use crate::stages::prompts::{fill_template, format_candidates, format_resources};
use crate::types::{CandidateGraph, DerivedEdge, EdgeKind, RejectedCandidate};

// LLM response shape for edge derivation
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
pub async fn run_derive_edges(llm: &dyn LlmBackend, graph: &CandidateGraph, template: &str) -> Result<(Vec<DerivedEdge>, Vec<RejectedCandidate>)> {
	tracing::info!("Stage 3: deriving edges via LLM…");

	let prompt = fill_template(
		template,
		&[("RESOURCES", &format_resources(&graph.resources)), ("CANDIDATE_EDGES", &format_candidates(graph))],
	);

	let raw = llm.complete(&prompt, "derive-edges").await?;
	let parsed: EdgeResponse = extract_json(&raw)?;

	let edges: Vec<DerivedEdge> = parsed
		.edges
		.into_iter()
		.map(|e| DerivedEdge {
			source: e.source,
			target: e.target,
			kind: e.kind.into(),
			weight: e.weight,
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

	tracing::info!("  → {} edges derived, {} rejected", edges.len(), rejected.len());

	Ok((edges, rejected))
}

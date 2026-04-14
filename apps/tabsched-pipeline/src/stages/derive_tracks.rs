/// Stage 4: LLM track grouping.
///
/// Changes from v1: StageError classification, per-call deadline.
use serde::Deserialize;
use tokio::time::timeout;
use tracing::instrument;

use crate::error::StageError;
use crate::llm::{extract_json, LlmBackend};
use crate::stages::prompts::{fill_template, format_resources_and_edges};
use crate::types::{CandidateGraph, DerivedEdge, DerivedTrack, TopologyChange};

pub const STAGE_TIMEOUT_SECS: u64 = 300;

#[derive(Deserialize)]
struct TrackResponse {
	tracks: Vec<TrackItem>,
	#[serde(default)]
	changes_from_current: Vec<ChangeItem>,
}

#[derive(Deserialize)]
struct TrackItem {
	label: String,
	parent: Option<String>,
	target: u32,
	#[serde(default)]
	resource_labels: Vec<String>,
	rationale: String,
}

#[derive(Deserialize)]
struct ChangeItem {
	kind: String,
	description: String,
}

#[instrument(
    name = "derive_tracks",
    skip_all,
    fields(
        window_size,
        resource_count = tracing::field::Empty,
        edge_count = tracing::field::Empty,
        has_previous = tracing::field::Empty
    )
)]
pub async fn run_derive_tracks(
	llm: &dyn LlmBackend,
	graph: &CandidateGraph,
	edges: &[DerivedEdge],
	current_tracks: Option<&serde_json::Value>,
	window_size: u32,
	template: &str,
) -> Result<(Vec<DerivedTrack>, Vec<TopologyChange>), StageError> {
	tracing::info!("Stage 4: deriving tracks via LLM");
	let span = tracing::Span::current();

	span.record("resource_count", graph.resources.len());
	span.record("edge_count", edges.len());
	span.record("has_previous", &current_tracks.is_some());

	let current_tracks_str = current_tracks.map(summarize_tracks).unwrap_or_else(|| "None".into());

	let prompt = fill_template(
		template,
		&[
			("RESOURCES_AND_EDGES", &format_resources_and_edges(&graph.resources, edges)),
			("CURRENT_TRACKS", &current_tracks_str),
			("WINDOW_SIZE", &window_size.to_string()),
		],
	);

	tracing::debug!(len = current_tracks_str.len(), has_previous = current_tracks.is_some(), "derive-tracks: prior state");
	let raw = timeout(std::time::Duration::from_secs(STAGE_TIMEOUT_SECS), llm.complete(&prompt, "derive-tracks"))
		.await
		.map_err(|_| StageError::retryable(anyhow::anyhow!("derive-tracks LLM call timed out")))?
		.map_err(|e| StageError::retryable(anyhow::anyhow!("LLM complete error: {}", e)))?;

	tracing::debug!(len = raw.len(), "derive-tracks: LLM raw output");

	let parsed: TrackResponse = extract_json(&raw).map_err(|e| StageError::retryable(anyhow::anyhow!("derive-tracks: LLM returned unparseable JSON: {}", e)))?;

	let tracks: Vec<DerivedTrack> = parsed
		.tracks
		.into_iter()
		.map(|t| DerivedTrack {
			label: t.label,
			parent: t.parent,
			target: t.target.max(1),
			resource_labels: t.resource_labels,
			derived_by: "llm".into(),
			rationale: t.rationale,
		})
		.collect();

	let changes: Vec<TopologyChange> = parsed
		.changes_from_current
		.into_iter()
		.map(|c| TopologyChange {
			kind: c.kind,
			description: c.description,
		})
		.collect();

	tracing::info!(tracks = tracks.len(), "Stage 4 complete");

	Ok((tracks, changes))
}

fn summarize_tracks(t: &serde_json::Value) -> String {
	let mut out = String::new();
	if let Some(tracks) = t.get("tracks").and_then(|v| v.as_array()) {
		for tr in tracks.iter().take(10) {
			let label = tr.get("label").and_then(|v| v.as_str()).unwrap_or("?");
			let parent = tr.get("parent").and_then(|v| v.as_str()).unwrap_or("None");
			let target = tr.get("target").and_then(|v| v.as_u64()).unwrap_or(0);
			out.push_str(&format!("- {} (target={}, parent={})\n", label, target, parent));
		}
	}
	if out.is_empty() {
		"None".into()
	} else {
		out
	}
}

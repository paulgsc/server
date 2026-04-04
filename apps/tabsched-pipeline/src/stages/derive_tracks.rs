use anyhow::Result;
use serde::Deserialize;
use tracing::instrument;

use crate::llm::{extract_json, LlmBackend};
use crate::stages::prompts::{fill_template, format_resources_and_edges};
use crate::types::{CandidateGraph, DerivedEdge, DerivedTrack, TopologyChange};

// LLM response shape for track grouping
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
	#[allow(dead_code)]
	#[serde(default)]
	derived_by: String,
	rationale: String,
}

#[derive(Deserialize)]
struct ChangeItem {
	kind: String,
	description: String,
}

#[instrument(skip(llm, graph, edges, current_tracks, template))]
pub async fn run_derive_tracks(
	llm: &dyn LlmBackend,
	graph: &CandidateGraph,
	edges: &[DerivedEdge],
	current_tracks: Option<&serde_json::Value>,
	window_size: u32,
	template: &str,
) -> Result<(Vec<DerivedTrack>, Vec<TopologyChange>)> {
	tracing::info!("Stage 4: deriving tracks via LLM…");

	let current_tracks_str = current_tracks
		.map(|t| serde_json::to_string_pretty(t).unwrap_or_default())
		.unwrap_or_else(|| "None — this is the first pipeline run.".into());

	let prompt = fill_template(
		template,
		&[
			("RESOURCES_AND_EDGES", &format_resources_and_edges(&graph.resources, edges)),
			("CURRENT_TRACKS", &current_tracks_str),
			("WINDOW_SIZE", &window_size.to_string()),
		],
	);

	let raw = llm.complete(&prompt, "derive-tracks").await?;
	let parsed: TrackResponse = extract_json(&raw)?;

	let tracks: Vec<DerivedTrack> = parsed
		.tracks
		.into_iter()
		.map(|t| DerivedTrack {
			label: t.label,
			parent: t.parent,
			target: t.target,
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

	tracing::info!("  → {} tracks derived", tracks.len());

	Ok((tracks, changes))
}

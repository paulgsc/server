use serde::{Deserialize, Serialize};
use ws_events::tabsched::Domain;

// ── Stage 1: embedded resource ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedResource {
	pub tab_id: i64,
	pub url: String,
	/// Short TOML-safe slug derived from the URL.
	pub label: String,
	pub domain: Domain,
	pub content_kind: String,
	pub title: String,
	pub summary: String,
	pub headings: Vec<String>,
	pub keywords: Vec<String>,
	/// Raw embedding vector.  Stripped before writing CandidateGraph output.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub embedding: Vec<f32>,
}

// ── Stage 2: candidate graph ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateEdge {
	pub source_label: String,
	pub target_label: String,
	/// Cosine similarity, 0.0–1.0.
	pub similarity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateGraph {
	pub embedded_at: String,
	pub embed_model: String,
	pub similarity_threshold: f32,
	pub resources: Vec<EmbeddedResource>,
	pub candidate_edges: Vec<CandidateEdge>,
}

// ── Stage 3: derived edges ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
	Similar,
	Reinforces,
	Overlaps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedEdge {
	pub source: String,
	pub target: String,
	pub kind: EdgeKind,
	pub weight: f32,
	pub reason: String,
	pub derived_by: String,
	pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedCandidate {
	pub source: String,
	pub target: String,
	pub reason: String,
}

// ── Stage 4: derived tracks ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedTrack {
	pub label: String,
	pub parent: Option<String>,
	pub target: u32,
	pub resource_labels: Vec<String>,
	pub derived_by: String,
	pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyChange {
	pub kind: String,
	pub description: String,
}

// ── Pipeline output ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineOutput {
	pub run_id: String,
	pub run_at: String,
	pub model: String,
	pub embed_model: String,
	pub window_size: u32,
	pub resources: Vec<EmbeddedResource>,
	pub edges: Vec<DerivedEdge>,
	pub tracks: Vec<DerivedTrack>,
	pub changes_from_current: Vec<TopologyChange>,
	pub rejected_candidates: Vec<RejectedCandidate>,
}

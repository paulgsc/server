///
/// Templates use `{{VARIABLE}}` syntax matching the existing .md files.
use crate::types::{CandidateGraph, DerivedEdge, EmbeddedResource};

pub fn fill_template(template: &str, vars: &[(&str, &str)]) -> String {
	let mut out = template.to_string();
	for (key, val) in vars {
		out = out.replace(&format!("{{{{{}}}}}", key), val);
	}
	out
}

// ── Prompt formatters ─────────────────────────────────────────────────────

pub fn format_resources(resources: &[EmbeddedResource]) -> String {
	resources
		.iter()
		.map(|r| {
			format!(
				"Resource: {}\n  URL: {}\n  Domain: {}\n  Kind: {}\n  Title: {}\n  Summary: {}\n  Headings: {}\n  Keywords: {}",
				r.label,
				r.url,
				r.domain,
				r.content_kind,
				r.title,
				&r.summary.chars().take(200).collect::<String>(),
				r.headings.iter().take(5).cloned().collect::<Vec<_>>().join(" | "),
				r.keywords.iter().take(10).cloned().collect::<Vec<_>>().join(", "),
			)
		})
		.collect::<Vec<_>>()
		.join("\n\n")
}

pub fn format_candidates(graph: &CandidateGraph) -> String {
	if graph.candidate_edges.is_empty() {
		return "No candidate pairs above similarity threshold.".into();
	}
	graph
		.candidate_edges
		.iter()
		.map(|e| format!("{} ↔ {}  similarity={:.3}", e.source_label, e.target_label, e.similarity))
		.collect::<Vec<_>>()
		.join("\n")
}

pub fn format_resources_and_edges(resources: &[EmbeddedResource], edges: &[DerivedEdge]) -> String {
	let resource_lines = resources
		.iter()
		.map(|r| format!("{}  [{}]  \"{}\"", r.label, r.domain, r.title))
		.collect::<Vec<_>>()
		.join("\n");

	let edge_lines = if edges.is_empty() {
		"No edges derived.".into()
	} else {
		edges
			.iter()
			.map(|e| format!("{} --[{:?} w={:.1}]--> {}", e.source, e.kind, e.weight, e.target))
			.collect::<Vec<_>>()
			.join("\n")
	};

	format!("Resources:\n{}\n\nEdges:\n{}", resource_lines, edge_lines)
}

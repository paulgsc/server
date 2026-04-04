///
/// Concurrency model:
///   - OpenAI: single batched request (the API accepts arrays).
///   - Ollama: individual requests fired concurrently with join_all
///     (Ollama's /api/embeddings does not support batching).
///
/// Both paths return Vec<Vec<f32>> in input order.
///
/// After embedding, cosine similarity is computed in a pure O(n²) loop
/// (n is small — a single browser session is typically < 100 tabs).
use anyhow::{Context, Result};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

use crate::stages::label::derive_label;
use crate::types::{CandidateEdge, CandidateGraph, EmbeddedResource, TabCapture};

// ── Config ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum EmbedProvider {
	/// OpenAI text-embedding-3-small (or any batching-compatible endpoint).
	OpenAI { api_key: String, base_url: String, model: String },
	/// Ollama nomic-embed-text (or any single-prompt embedding endpoint).
	Ollama { host: String, model: String },
}

impl EmbedProvider {
	pub fn model_name(&self) -> &str {
		match self {
			EmbedProvider::OpenAI { model, .. } => model,
			EmbedProvider::Ollama { model, .. } => model,
		}
	}
}

// ── OpenAI batch embedding ────────────────────────────────────────────────

#[derive(Serialize)]
struct OpenAIEmbedRequest<'a> {
	model: &'a str,
	input: &'a [String],
}

#[derive(Deserialize)]
struct OpenAIEmbedResponse {
	data: Vec<OpenAIEmbedDatum>,
}

#[derive(Deserialize)]
struct OpenAIEmbedDatum {
	embedding: Vec<f32>,
}

async fn embed_openai(client: &reqwest::Client, api_key: &str, base_url: &str, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
	let url = format!("{}/embeddings", base_url);
	let resp = client
		.post(&url)
		.bearer_auth(api_key)
		.json(&OpenAIEmbedRequest { model, input: texts })
		.send()
		.await
		.context("OpenAI embed request failed")?;

	if !resp.status().is_success() {
		let status = resp.status();
		let body = resp.text().await.unwrap_or_default();
		anyhow::bail!("OpenAI embed error {}: {}", status, body);
	}

	let parsed: OpenAIEmbedResponse = resp.json().await.context("parse OpenAI embed response")?;
	Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
}

// ── Ollama concurrent embedding ───────────────────────────────────────────

#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
	model: &'a str,
	prompt: &'a str,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
	embedding: Vec<f32>,
}

async fn embed_single_ollama(client: &reqwest::Client, host: &str, model: &str, text: &str) -> Result<Vec<f32>> {
	let url = format!("{}/api/embeddings", host);
	let resp = client
		.post(&url)
		.json(&OllamaEmbedRequest { model, prompt: text })
		.send()
		.await
		.context("Ollama embed request failed")?;

	if !resp.status().is_success() {
		let status = resp.status();
		let body = resp.text().await.unwrap_or_default();
		anyhow::bail!("Ollama embed error {}: {}", status, body);
	}

	let parsed: OllamaEmbedResponse = resp.json().await.context("parse Ollama embed response")?;
	Ok(parsed.embedding)
}

async fn embed_ollama(client: &reqwest::Client, host: &str, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
	// Fire all requests concurrently — Ollama serialises on its end anyway,
	// but this avoids head-of-line blocking for network latency.
	let futures: Vec<_> = texts.iter().map(|t| embed_single_ollama(client, host, model, t)).collect();

	let results = join_all(futures).await;
	results.into_iter().collect::<Result<Vec<_>>>()
}

// ── Text to embed ─────────────────────────────────────────────────────────

fn build_embed_text(capture: &TabCapture) -> String {
	let c = &capture.content;
	[
		c.title.clone(),
		c.headings.iter().take(5).cloned().collect::<Vec<_>>().join(" "),
		c.keywords.join(" "),
		c.summary.chars().take(200).collect::<String>(),
	]
	.into_iter()
	.filter(|s| !s.is_empty())
	.collect::<Vec<_>>()
	.join(" | ")
}

// ── Cosine similarity ─────────────────────────────────────────────────────

fn cosine(a: &[f32], b: &[f32]) -> f32 {
	let (mut dot, mut norm_a, mut norm_b) = (0.0f32, 0.0f32, 0.0f32);
	for (x, y) in a.iter().zip(b.iter()) {
		dot += x * y;
		norm_a += x * x;
		norm_b += y * y;
	}
	dot / (norm_a.sqrt() * norm_b.sqrt())
}

// ── Public entry point ────────────────────────────────────────────────────

#[instrument(skip(client, provider, captures), fields(n = captures.len()))]
pub async fn run_embed_stage(client: &reqwest::Client, provider: &EmbedProvider, captures: &[TabCapture], similarity_threshold: f32) -> Result<CandidateGraph> {
	info!("Stage 2: embedding {} resources", captures.len());

	let texts: Vec<String> = captures.iter().map(build_embed_text).collect();

	let embeddings = match provider {
		EmbedProvider::OpenAI { api_key, base_url, model } => embed_openai(client, api_key, base_url, model, &texts).await?,
		EmbedProvider::Ollama { host, model } => embed_ollama(client, host, model, &texts).await?,
	};

	// Build EmbeddedResource objects
	let resources: Vec<EmbeddedResource> = captures
		.iter()
		.zip(embeddings.iter())
		.map(|(c, emb)| EmbeddedResource {
			tab_id: c.tab_id,
			url: c.url.clone(),
			label: derive_label(c),
			domain: c.domain.clone(),
			content_kind: c.content.kind.0.clone(),
			title: c.content.title.clone(),
			summary: c.content.summary.clone(),
			headings: c.content.headings.clone(),
			keywords: c.content.keywords.clone(),
			embedding: emb.clone(),
		})
		.collect();

	// Compute candidate edges (upper triangle)
	let mut candidate_edges: Vec<CandidateEdge> = vec![];
	for i in 0..resources.len() {
		for j in (i + 1)..resources.len() {
			let sim = cosine(&resources[i].embedding, &resources[j].embedding);
			if sim >= similarity_threshold {
				candidate_edges.push(CandidateEdge {
					source_label: resources[i].label.clone(),
					target_label: resources[j].label.clone(),
					similarity: (sim * 1000.0).round() / 1000.0,
				});
			}
		}
	}

	// Sort descending by similarity
	candidate_edges.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());

	info!("Found {} candidate edges above threshold {}", candidate_edges.len(), similarity_threshold);

	// Strip embeddings before serialising — they're large and unused downstream
	let resources_stripped = resources
		.into_iter()
		.map(|mut r| {
			r.embedding = vec![];
			r
		})
		.collect();

	Ok(CandidateGraph {
		embedded_at: chrono::Utc::now().to_rfc3339(),
		embed_model: provider.model_name().to_string(),
		similarity_threshold,
		resources: resources_stripped,
		candidate_edges,
	})
}

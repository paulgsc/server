///
/// Stage 2: embedding + candidate edge computation.
/// Changes from v1:
///   - All returns are StageError, not anyhow::Error.
///   - Input guardrails: max captures, max text bytes per capture.
///   - Ollama concurrent path is bounded by a semaphore (MAX_CONCURRENT_EMBEDS)
///     to prevent flooding the local model.
///   - Per-request timeout via tokio::time::timeout (not the global reqwest timeout).
///   - Network errors → Retryable; schema errors → Permanent.
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::{sync::Semaphore, time::timeout};
use tracing::{info, instrument, warn};

use crate::error::StageError;
use crate::stages::label::derive_label;
use crate::types::{CandidateEdge, CandidateGraph, EmbeddedResource};
use ws_events::tabsched::TabCapture;

// ── Guardrails ────────────────────────────────────────────────────────────

/// Hard cap on captures processed per session.
/// Sessions larger than this are truncated (not rejected) — we embed
/// the first N and log a warning.
pub const MAX_CAPTURES: usize = 200;

/// Hard cap on embed text bytes per capture (pre-request).
/// Text is truncated to this length, not rejected.
pub const MAX_EMBED_TEXT_BYTES: usize = 2_048;

/// Maximum concurrent Ollama embed requests.
/// Ollama serialises internally anyway; this cap prevents spawning
/// hundreds of tasks against a single-threaded model server.
pub const MAX_CONCURRENT_EMBEDS: usize = 8;

/// Per-request timeout for a single embed call.
pub const EMBED_TIMEOUT_SECS: u64 = 30;

// ── Config ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum EmbedProvider {
	OpenAI { api_key: String, base_url: String, model: String },
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

async fn embed_openai(client: &reqwest::Client, api_key: &str, base_url: &str, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, StageError> {
	let url = format!("{}/embeddings", base_url);

	let fut = client.post(&url).bearer_auth(api_key).json(&OpenAIEmbedRequest { model, input: texts }).send();

	let resp = timeout(std::time::Duration::from_secs(EMBED_TIMEOUT_SECS * texts.len().max(1) as u64), fut)
		.await
		.map_err(|_| StageError::retryable(anyhow::anyhow!("OpenAI embed request timed out")))?
		.map_err(|e| StageError::retryable(anyhow::anyhow!("OpenAI embed request: {}", e)))?;

	if resp.status().is_server_error() {
		return Err(StageError::retryable(anyhow::anyhow!("OpenAI embed server error: {}", resp.status())));
	}
	if !resp.status().is_success() {
		let body = resp.text().await.unwrap_or_default();
		return Err(StageError::permanent(anyhow::anyhow!("OpenAI embed client error: {}", body)));
	}

	let parsed: OpenAIEmbedResponse = resp
		.json()
		.await
		.map_err(|e| StageError::permanent(anyhow::anyhow!("parse OpenAI embed response: {}", e)))?;

	if parsed.data.len() != texts.len() {
		return Err(StageError::permanent(anyhow::anyhow!(
			"OpenAI returned {} embeddings for {} inputs",
			parsed.data.len(),
			texts.len()
		)));
	}

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

async fn embed_single_ollama(client: &reqwest::Client, host: &str, model: &str, text: &str, sem: Arc<Semaphore>) -> Result<Vec<f32>, StageError> {
	let _permit = sem.acquire_owned().await.map_err(|e| StageError::retryable(anyhow::anyhow!("semaphore closed: {}", e)))?;

	let url = format!("{}/api/embeddings", host);

	let resp = timeout(
		std::time::Duration::from_secs(EMBED_TIMEOUT_SECS),
		client.post(&url).json(&OllamaEmbedRequest { model, prompt: text }).send(),
	)
	.await
	.map_err(|_| StageError::retryable(anyhow::anyhow!("Ollama embed timed out")))?
	.map_err(|e| StageError::retryable(anyhow::anyhow!("Ollama embed request: {}", e)))?;

	if resp.status().is_server_error() {
		return Err(StageError::retryable(anyhow::anyhow!("Ollama server error: {}", resp.status())));
	}
	if !resp.status().is_success() {
		let body = resp.text().await.unwrap_or_default();
		return Err(StageError::permanent(anyhow::anyhow!("Ollama embed client error: {}", body)));
	}

	let parsed: OllamaEmbedResponse = resp
		.json()
		.await
		.map_err(|e| StageError::permanent(anyhow::anyhow!("parse Ollama embed response: {}", e)))?;

	Ok(parsed.embedding)
}

async fn embed_ollama(client: &reqwest::Client, host: &str, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, StageError> {
	let sem = Arc::new(Semaphore::new(MAX_CONCURRENT_EMBEDS));

	let futures: Vec<_> = texts.iter().map(|t| embed_single_ollama(client, host, model, t, sem.clone())).collect();

	let results = join_all(futures).await;

	// Collect, propagating first error.
	results.into_iter().collect::<Result<Vec<_>, _>>()
}

// ── Text preparation ──────────────────────────────────────────────────────

fn build_embed_text(capture: &TabCapture) -> String {
	let c = &capture.content;
	let raw = [
		c.title.clone(),
		c.headings.iter().take(5).cloned().collect::<Vec<_>>().join(" "),
		c.keywords.join(" "),
		c.summary.chars().take(200).collect::<String>(),
	]
	.into_iter()
	.filter(|s| !s.is_empty())
	.collect::<Vec<_>>()
	.join(" | ");

	// Truncate to byte limit rather than rejecting — embed text is not
	// security-sensitive, and a long summary doesn't make a session invalid.
	if raw.len() > MAX_EMBED_TEXT_BYTES {
		warn!(
				url = %capture.url,
				original_bytes = raw.len(),
				limit = MAX_EMBED_TEXT_BYTES,
				"embed text truncated"
		);
		raw[..MAX_EMBED_TEXT_BYTES].to_string()
	} else {
		raw
	}
}

// ── Cosine similarity ─────────────────────────────────────────────────────

fn cosine(a: &[f32], b: &[f32]) -> f32 {
	let (mut dot, mut norm_a, mut norm_b) = (0.0f32, 0.0f32, 0.0f32);
	for (x, y) in a.iter().zip(b.iter()) {
		dot += x * y;
		norm_a += x * x;
		norm_b += y * y;
	}
	let denom = norm_a.sqrt() * norm_b.sqrt();
	if denom == 0.0 {
		0.0
	} else {
		dot / denom
	}
}

// ── Public entry point ────────────────────────────────────────────────────

#[instrument(skip(client, provider, captures), fields(n = captures.len()))]
pub async fn run_embed_stage(client: &reqwest::Client, provider: &EmbedProvider, captures: &[TabCapture], similarity_threshold: f32) -> Result<CandidateGraph, StageError> {
	// ── Input guardrails ──────────────────────────────────────────────────
	if captures.is_empty() {
		return Err(StageError::poison(anyhow::anyhow!("no captures to embed after extraction_ok filter")));
	}

	let captures = if captures.len() > MAX_CAPTURES {
		warn!(total = captures.len(), limit = MAX_CAPTURES, "session exceeds MAX_CAPTURES — truncating");
		&captures[..MAX_CAPTURES]
	} else {
		captures
	};

	info!("Stage 2: embedding {} resources", captures.len());

	let texts: Vec<String> = captures.iter().map(build_embed_text).collect();

	let embeddings = match provider {
		EmbedProvider::OpenAI { api_key, base_url, model } => embed_openai(client, api_key, base_url, model, &texts).await?,
		EmbedProvider::Ollama { host, model } => embed_ollama(client, host, model, &texts).await?,
	};

	// Sanity check: provider must return one embedding per input.
	if embeddings.len() != captures.len() {
		return Err(StageError::permanent(anyhow::anyhow!(
			"embedding count mismatch: got {}, expected {}",
			embeddings.len(),
			captures.len()
		)));
	}

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

	// Compute candidate edges (upper triangle, O(n²), n is small).
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

	candidate_edges.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());

	info!("Found {} candidate edges above threshold {}", candidate_edges.len(), similarity_threshold);

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

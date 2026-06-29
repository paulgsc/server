///
/// Works with Ollama (`ollama serve`), LM Studio, vLLM, or the real
/// OpenAI API.  The endpoint and model are fully configurable.
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::LlmBackend;

#[derive(Debug, Clone)]
pub struct LocalLlmConfig {
	/// Base URL, e.g. "http://localhost:11434/v1" for Ollama.
	pub base_url: String,
	/// Model identifier, e.g. "llama3", "mistral", "phi3".
	pub model: String,
	/// Optional Bearer token (required for real OpenAI).
	pub api_key: Option<String>,
	pub max_tokens: u32,
	pub temperature: f32,
}

impl Default for LocalLlmConfig {
	fn default() -> Self {
		Self {
			base_url: "http://localhost:11434/v1".into(),
			model: "llama3".into(),
			api_key: None,
			max_tokens: 4096,
			temperature: 0.1,
		}
	}
}

pub struct LocalLlm {
	config: LocalLlmConfig,
	client: reqwest::Client,
}

impl LocalLlm {
	pub fn new(config: LocalLlmConfig) -> Self {
		let client = reqwest::Client::builder()
			.timeout(std::time::Duration::from_secs(300))
			.build()
			.expect("failed to build reqwest client");
		Self { config, client }
	}
}

// ── OpenAI-compat request/response shapes ────────────────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
	model: &'a str,
	messages: Vec<Message<'a>>,
	max_tokens: u32,
	temperature: f32,
}

#[derive(Serialize)]
struct Message<'a> {
	role: &'a str,
	content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
	choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
	message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
	content: String,
}

#[async_trait]
impl LlmBackend for LocalLlm {
	#[instrument(skip(self, prompt), fields(label = %label, model = %self.config.model))]
	async fn complete(&self, prompt: &str, label: &str) -> Result<String> {
		let url = format!("{}/chat/completions", self.config.base_url);

		let mut req = self.client.post(&url).json(&ChatRequest {
			model: &self.config.model,
			messages: vec![
				Message {
					role: "system",
					content: "You are a strict JSON generator. You must output valid JSON only.",
				},
				Message { role: "user", content: prompt },
			],
			max_tokens: self.config.max_tokens,
			temperature: self.config.temperature,
		});

		if let Some(key) = &self.config.api_key {
			req = req.bearer_auth(key);
		}

		let resp = req.send().await.context("HTTP request to LLM endpoint failed")?;

		if !resp.status().is_success() {
			let status = resp.status();
			let body = resp.text().await.unwrap_or_default();
			anyhow::bail!("LLM endpoint returned {}: {}", status, body);
		}

		let chat: ChatResponse = resp.json().await.context("failed to parse LLM JSON response")?;

		chat
			.choices
			.into_iter()
			.next()
			.map(|c| c.message.content)
			.ok_or_else(|| anyhow::anyhow!("LLM response contained no choices"))
	}

	fn model_name(&self) -> &str {
		&self.config.model
	}
}

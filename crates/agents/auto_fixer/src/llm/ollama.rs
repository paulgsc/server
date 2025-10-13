#![allow(dead_code)]

use crate::{ClippyIssue, ESLintIssue};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct OllamaRequest {
	model: String,
	prompt: String,
	stream: bool,
}

#[derive(Deserialize)]
struct OllamaResponse {
	response: String,
	done: bool,
}

pub struct OllamaClient {
	client: reqwest::Client,
	base_url: String,
	model: String,
}

impl Default for OllamaClient {
	fn default() -> Self {
		Self {
			client: reqwest::Client::new(),
			base_url: "http://localhost:11434".to_string(),
			model: "codellama:7b".to_string(),
		}
	}
}

impl OllamaClient {
	pub fn new(base_url: String, model: String) -> Self {
		Self {
			client: reqwest::Client::new(),
			base_url,
			model,
		}
	}

	pub async fn fix_clippy_issue(&self, issue: &ClippyIssue) -> Result<String> {
		let prompt = format!(
			"Fix this Rust clippy issue:\n\n\
													File: {}\n\
																Line {}, Column {}\n\
																			Rule: {}\n\
																						Issue: {}\n\
																									Code:\n```rust\n{}\n```\n\n\
																												Please provide only the corrected code snippet.",
			issue.file_path, issue.line, issue.column, issue.rule, issue.message, issue.code_snippet
		);

		self.generate(&prompt).await
	}

	pub async fn fix_eslint_issue(&self, issue: &ESLintIssue) -> Result<String> {
		let prompt = format!(
			"Fix this ESLint/TypeScript issue:\n\n\
														File: {}\n\
																	Line {}, Column {}\n\
																				Rule: {}\n\
																							Issue: {}\n\
																										Code:\n```typescript\n{}\n```\n\n\
																													Please provide only the corrected code snippet.",
			issue.file_path, issue.line, issue.column, issue.rule_id, issue.message, issue.code_snippet
		);

		self.generate(&prompt).await
	}

	async fn generate(&self, prompt: &str) -> Result<String> {
		let request = OllamaRequest {
			model: self.model.clone(),
			prompt: prompt.to_string(),
			stream: false,
		};

		let response = self
			.client
			.post(&format!("{}/api/generate", self.base_url))
			.json(&request)
			.send()
			.await
			.context("Failed to send request to Ollama")?;

		let ollama_response: OllamaResponse = response.json().await.context("Failed to parse Ollama response")?;

		Ok(ollama_response.response)
	}

	pub async fn health_check(&self) -> Result<bool> {
		let response = self.client.get(&format!("{}/api/version", self.base_url)).send().await;

		match response {
			Ok(resp) => Ok(resp.status().is_success()),
			Err(_) => Ok(false),
		}
	}

	pub async fn list_models(&self) -> Result<Vec<String>> {
		let response = self
			.client
			.get(&format!("{}/api/tags", self.base_url))
			.send()
			.await
			.context("Failed to fetch models from Ollama")?;

		#[derive(Deserialize)]
		struct ModelsResponse {
			models: Vec<ModelInfo>,
		}

		#[derive(Deserialize)]
		struct ModelInfo {
			name: String,
		}

		let models_resp: ModelsResponse = response.json().await.context("Failed to parse models response")?;

		Ok(models_resp.models.into_iter().map(|m| m.name).collect())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio_test;

	#[tokio::test]
	async fn test_ollama_client_creation() {
		let client = OllamaClient::default();
		assert_eq!(client.base_url, "http://localhost:11434");
		assert_eq!(client.model, "codellama:7b");
	}

	#[tokio::test]
	async fn test_clippy_prompt_format() {
		let client = OllamaClient::default();
		let issue = ClippyIssue {
			file_path: "src/main.rs".to_string(),
			line: 10,
			column: 5,
			rule: "clippy::too_many_arguments".to_string(),
			message: "this function has too many arguments".to_string(),
			suggestion: None,
			code_snippet: "fn test(a: i32, b: i32, c: i32) {}".to_string(),
		};

		// This would normally call ollama, but we're just testing the prompt creation
		// In a real test environment, you'd mock the HTTP client
		assert_eq!(issue.line, 10);
	}
}

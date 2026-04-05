///
/// Two implementations:
///
///   LocalLlm   — HTTP call to any OpenAI-compat /v1/chat/completions
///                endpoint (Ollama, LM Studio, vLLM).
///
///   MarkdownDump — writes prompts to .md files on disk and reads
///                  responses from sibling .response.md files.
///                  Use this when you want to paste into a web LLM.
///
/// The trait is object-safe so callers hold Box<dyn LlmBackend>.
use anyhow::Result;
use async_trait::async_trait;

mod local;
mod markdown_dump;

pub use local::LocalLlm;
pub use markdown_dump::MarkdownDump;

// Re-export for convenience
pub use self::local::LocalLlmConfig;
pub use self::markdown_dump::MarkdownDumpConfig;

#[async_trait]
pub trait LlmBackend: Send + Sync {
	/// Submit `prompt` and return the raw text response.
	///
	/// For LocalLlm this is a blocking-on-async HTTP call.
	/// For MarkdownDump this writes the prompt, waits for a .response.md
	/// to appear (user pastes the answer), and returns its contents.
	async fn complete(&self, prompt: &str, label: &str) -> Result<String>;

	/// Human-readable name used in PipelineOutput.model.
	fn model_name(&self) -> &str;
}

/// Parse JSON from an LLM response, stripping markdown fences.
pub fn extract_json<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T> {
	let stripped = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();

	// Find first { or [
	let start = stripped
		.find(|c| c == '{' || c == '[')
		.ok_or_else(|| anyhow::anyhow!("No JSON object found in LLM response"))?;

	Ok(serde_json::from_str(&stripped[start..])?)
}

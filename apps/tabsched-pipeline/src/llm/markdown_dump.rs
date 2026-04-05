///
/// Instead of calling an LLM API, this backend:
///   1. Writes the prompt to  `<out_dir>/<label>.prompt.md`
///   2. Prints a message telling the user to paste the response
///   3. Polls for `<out_dir>/<label>.response.md` (created by the user)
///   4. Returns the file contents as the "LLM response"
///
/// This is the zero-cost fallback for environments with no local model
/// and no API key.  The user opens each .prompt.md in a browser LLM
/// (Claude.ai, ChatGPT, etc.) and saves the response alongside it.
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

use super::LlmBackend;

#[derive(Debug, Clone)]
pub struct MarkdownDumpConfig {
	/// Directory where prompt and response files are written.
	pub out_dir: PathBuf,
	/// How often to poll for the response file (seconds).
	pub poll_interval_secs: u64,
	/// Maximum wait time before giving up (seconds). 0 = wait forever.
	pub timeout_secs: u64,
}

impl Default for MarkdownDumpConfig {
	fn default() -> Self {
		Self {
			out_dir: PathBuf::from("pipeline-prompts"),
			poll_interval_secs: 5,
			timeout_secs: 0,
		}
	}
}

pub struct MarkdownDump {
	config: MarkdownDumpConfig,
}

impl MarkdownDump {
	pub fn new(config: MarkdownDumpConfig) -> Self {
		Self { config }
	}

	fn prompt_path(&self, label: &str) -> PathBuf {
		self.config.out_dir.join(format!("{}.prompt.md", label))
	}

	fn response_path(&self, label: &str) -> PathBuf {
		self.config.out_dir.join(format!("{}.response.md", label))
	}
}

#[async_trait]
impl LlmBackend for MarkdownDump {
	async fn complete(&self, prompt: &str, label: &str) -> Result<String> {
		tokio::fs::create_dir_all(&self.config.out_dir).await.context("failed to create prompt output directory")?;

		let prompt_path = self.prompt_path(label);
		let response_path = self.response_path(label);

		// Write prompt
		tokio::fs::write(&prompt_path, prompt)
			.await
			.with_context(|| format!("failed to write prompt to {:?}", prompt_path))?;

		info!(
			"\n\
             ╔══════════════════════════════════════════════════════════╗\n\
             ║  MANUAL LLM STEP REQUIRED                                ║\n\
             ╠══════════════════════════════════════════════════════════╣\n\
             ║  Stage : {label:<50}║\n\
             ║  Prompt: {prompt:<50}║\n\
             ║  1. Open the prompt file in your browser LLM.            ║\n\
             ║  2. Copy the response into the response file.            ║\n\
             ║  3. Save — the pipeline will continue automatically.     ║\n\
             ╚══════════════════════════════════════════════════════════╝",
			label = label,
			prompt = prompt_path.display(),
		);
		eprintln!(
			"\n[pipeline] Prompt written → {}\n[pipeline] Waiting for response → {}\n",
			prompt_path.display(),
			response_path.display(),
		);

		// Poll for response
		let mut elapsed = 0u64;
		loop {
			if response_path.exists() {
				let content = tokio::fs::read_to_string(&response_path)
					.await
					.with_context(|| format!("failed to read response from {:?}", response_path))?;
				eprintln!("[pipeline] Response received for stage: {}", label);
				return Ok(content);
			}

			if self.config.timeout_secs > 0 && elapsed >= self.config.timeout_secs {
				anyhow::bail!("Timed out waiting for response file: {}", response_path.display());
			}

			sleep(Duration::from_secs(self.config.poll_interval_secs)).await;
			elapsed += self.config.poll_interval_secs;

			if elapsed % 60 == 0 {
				warn!("[pipeline] Still waiting for {} ({} s elapsed)…", response_path.display(), elapsed);
			}
		}
	}

	fn model_name(&self) -> &str {
		"manual-paste"
	}
}

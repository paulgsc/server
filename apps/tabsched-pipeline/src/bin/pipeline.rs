///
/// Stages:
///   1. Read CaptureSession JSON (from file or stdin).
///   2. Embed resources (OpenAI or Ollama, concurrent).
///   3. Derive edges   (LLM call — sequential, prompt-bounded).
///   4. Derive tracks  (LLM call — sequential, prompt-bounded).
///   5. Write pipeline-output-<date>.json + topology-<date>.toml.
///
/// LLM backend is selected via --llm-mode:
///   local   → POST to any OpenAI-compat endpoint (default: Ollama).
///   dump    → Write prompts to disk; wait for .response.md files.
///
/// Designed to run as a standalone, non-happy-path bin crate.
/// It does NOT import anything from the Axum server crate.
use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use tabsched_pipeline::{
	llm::{LlmBackend, LocalLlm, LocalLlmConfig, MarkdownDump, MarkdownDumpConfig},
	stages::{
		derive_edges::run_derive_edges,
		derive_tracks::run_derive_tracks,
		embed::{run_embed_stage, EmbedProvider},
		toml_gen::generate_toml,
	},
	types::{CaptureSession, PipelineOutput},
};

// ── CLI ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, ValueEnum)]
enum LlmMode {
	/// POST to a local OpenAI-compat endpoint (Ollama, vLLM, etc.)
	Local,
	/// Write prompts to disk; read .response.md files written by the user.
	Dump,
}

#[derive(Debug, Clone, ValueEnum)]
enum EmbedMode {
	/// Ollama nomic-embed-text (default).
	Ollama,
	/// OpenAI text-embedding-3-small.
	Openai,
}

#[derive(Debug, Parser)]
#[command(
    name = "tabsched-pipeline",
    about = "Offline embedding + LLM edge/track derivation for tabsched",
    long_about = None,
)]
struct Args {
	/// Path to CaptureSession JSON (produced by the browser extension or the
	/// Axum /capture endpoint).  Use '-' to read from stdin.
	#[arg(value_name = "CAPTURE_JSON")]
	capture_json: PathBuf,

	/// Optional existing topology JSON (previous pipeline output).
	/// When provided, the LLM receives it as a soft constraint.
	#[arg(long, value_name = "CURRENT_TRACKS_JSON")]
	current_tracks: Option<PathBuf>,

	/// Directory containing derive-edges.md and derive-tracks.md prompt templates.
	#[arg(long, env = "PIPELINE_PROMPTS_DIR")]
	prompts_dir: PathBuf,

	/// LLM backend selection.
	#[arg(long, value_enum, default_value = "local", env = "LLM_MODE")]
	llm_mode: LlmMode,

	/// Embedding provider.
	#[arg(long, value_enum, default_value = "ollama", env = "EMBED_PROVIDER")]
	embed_provider: EmbedMode,

	/// Ollama base URL (used for both embedding and LLM when mode = local).
	#[arg(long, default_value = "http://localhost:11434", env = "OLLAMA_HOST")]
	ollama_host: String,

	/// LLM model identifier (Ollama model name or OpenAI model string).
	#[arg(long, default_value = "llama3", env = "LLM_MODEL")]
	llm_model: String,

	/// OpenAI API key (required when --embed-provider=openai or using OpenAI LLM).
	#[arg(long, env = "OPENAI_API_KEY")]
	openai_api_key: Option<String>,

	/// Embedding model for OpenAI provider.
	#[arg(long, default_value = "text-embedding-3-small", env = "EMBED_MODEL")]
	embed_model: String,

	/// Cosine similarity threshold for candidate edge generation.
	#[arg(long, default_value = "0.65", env = "SIMILARITY_THRESHOLD")]
	similarity_threshold: f32,

	/// Scheduler window size (passed to the track grouping prompt).
	#[arg(long, default_value = "20", env = "WINDOW_SIZE")]
	window_size: u32,

	/// Directory for prompt and response files (--llm-mode=dump only).
	#[arg(long, default_value = "pipeline-prompts", env = "PROMPTS_OUT_DIR")]
	prompts_out_dir: PathBuf,

	/// Output directory for pipeline JSON and topology TOML.
	#[arg(long, default_value = ".", env = "PIPELINE_OUT_DIR")]
	out_dir: PathBuf,
}

// ── Main ───────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
	// Initialise logging — RUST_LOG controls verbosity (default: info)
	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
		.with_writer(std::io::stderr)
		.init();

	let args = Args::parse();

	// ── Read capture session ──────────────────────────────────────────────
	let capture_bytes = if args.capture_json.to_str() == Some("-") {
		use std::io::Read;
		let mut buf = Vec::new();
		std::io::stdin().read_to_end(&mut buf)?;
		buf
	} else {
		std::fs::read(&args.capture_json).with_context(|| format!("reading {:?}", args.capture_json))?
	};

	let session: CaptureSession = serde_json::from_slice(&capture_bytes).context("parsing CaptureSession JSON")?;

	info!(
			session_id = %session.session_id,
			total = session.captures.len(),
			"Loaded capture session"
	);

	let captures: Vec<_> = session.captures.into_iter().filter(|c| c.extraction_ok).collect();

	if captures.is_empty() {
		anyhow::bail!("No successfully extracted captures in session — nothing to do.");
	}
	info!("{} captures with extraction_ok=true", captures.len());

	// ── Optional current tracks ───────────────────────────────────────────
	let current_tracks: Option<serde_json::Value> = args
		.current_tracks
		.as_ref()
		.map(|p| {
			let raw = std::fs::read(p).with_context(|| format!("reading {:?}", p))?;
			serde_json::from_slice(&raw).context("parsing current-tracks JSON")
		})
		.transpose()?;

	// ── Load prompt templates ─────────────────────────────────────────────
	let edge_template = std::fs::read_to_string(args.prompts_dir.join("derive-edges.md")).context("reading derive-edges.md prompt template")?;
	let track_template = std::fs::read_to_string(args.prompts_dir.join("derive-tracks.md")).context("reading derive-tracks.md prompt template")?;

	// ── Build HTTP client (shared across embedding + LLM if local) ────────
	let http_client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build()?;

	// ── Stage 2: embed ────────────────────────────────────────────────────
	let embed_provider = match args.embed_provider {
		EmbedMode::Ollama => EmbedProvider::Ollama {
			host: args.ollama_host.clone(),
			model: "nomic-embed-text".into(),
		},
		EmbedMode::Openai => EmbedProvider::OpenAI {
			api_key: args.openai_api_key.clone().context("--openai-api-key required for embed-provider=openai")?,
			base_url: "https://api.openai.com/v1".into(),
			model: args.embed_model.clone(),
		},
	};

	let candidate_graph = run_embed_stage(&http_client, &embed_provider, &captures, args.similarity_threshold).await?;

	// ── Build LLM backend ─────────────────────────────────────────────────
	let llm: Box<dyn LlmBackend> = match args.llm_mode {
		LlmMode::Local => Box::new(LocalLlm::new(LocalLlmConfig {
			base_url: format!("{}/v1", args.ollama_host),
			model: args.llm_model.clone(),
			api_key: args.openai_api_key.clone(),
			max_tokens: 4096,
			temperature: 0.1,
		})),
		LlmMode::Dump => Box::new(MarkdownDump::new(MarkdownDumpConfig {
			out_dir: args.prompts_out_dir.clone(),
			poll_interval_secs: 5,
			timeout_secs: 0,
		})),
	};

	// ── Stage 3: derive edges ─────────────────────────────────────────────
	let (edges, rejected) = run_derive_edges(llm.as_ref(), &candidate_graph, &edge_template).await?;

	// ── Stage 4: derive tracks ────────────────────────────────────────────
	let (tracks, changes) = run_derive_tracks(llm.as_ref(), &candidate_graph, &edges, current_tracks.as_ref(), args.window_size, &track_template).await?;

	// ── Assemble output ───────────────────────────────────────────────────
	let run_at = chrono::Utc::now().to_rfc3339();
	let date_slug = &run_at[..10]; // "YYYY-MM-DD"

	let output = PipelineOutput {
		run_id: Uuid::new_v4().to_string(),
		run_at: run_at.clone(),
		model: llm.model_name().to_string(),
		embed_model: embed_provider.model_name().to_string(),
		window_size: args.window_size,
		resources: candidate_graph.resources.clone(),
		edges,
		tracks,
		changes_from_current: changes,
		rejected_candidates: rejected,
	};

	// ── Write outputs ─────────────────────────────────────────────────────
	std::fs::create_dir_all(&args.out_dir)?;

	let json_path = args.out_dir.join(format!("pipeline-output-{}.json", date_slug));
	std::fs::write(&json_path, serde_json::to_string_pretty(&output)?).with_context(|| format!("writing {:?}", json_path))?;
	info!("Pipeline output → {:?}", json_path);

	let toml_str = generate_toml(&output);
	let toml_path = args.out_dir.join(format!("topology-{}.toml", date_slug));
	std::fs::write(&toml_path, &toml_str).with_context(|| format!("writing {:?}", toml_path))?;
	info!("Topology TOML  → {:?}", toml_path);

	// ── Print summary to stdout ───────────────────────────────────────────
	println!("\nPipeline complete");
	println!("  Resources : {}", output.resources.len());
	println!("  Edges     : {} derived, {} rejected", output.edges.len(), output.rejected_candidates.len());
	println!("  Tracks    : {}", output.tracks.len());

	if !output.changes_from_current.is_empty() {
		println!("\nChanges from current topology:");
		for c in &output.changes_from_current {
			println!("  [{}] {}", c.kind, c.description);
		}
	}

	Ok(())
}

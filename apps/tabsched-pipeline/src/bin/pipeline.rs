///
/// Execution model:
///   main() boots N worker tasks (PIPELINE_WORKERS, default 2).
///   Each worker runs an infinite pull loop against the JetStream
///   consumer for `pipeline.jobs`.
///
///   Per job:
///     1. Fetch CaptureSession from Redis (written by Axum handler).
///     2. Validate + guard payload size.
///     3. run_embed_stage  → write artifact "embed" to Redis.
///     4. run_derive_edges → write artifact "edges" to Redis.
///     5. run_derive_tracks → write artifact "tracks" to Redis.
///     6. Assemble PipelineOutput → write artifact "output" to Redis.
///     7. ACK.
///
///   Failure dispatch (per StageError variant):
///     Retryable → NAK  (server redelivers after ack_wait)
///     Permanent → TERM + push_dlq  (exhausted retries or bad LLM JSON)
///     Poison    → TERM + push_dlq  (payload itself is broken)
///
///   Intermediate artifacts are written to Redis between stages.
///   A failure in stage 4 does NOT discard stage 2 output.
///   On redeliver the worker detects existing artifacts and skips
///   completed stages (idempotent resume).
///
/// IO contract:
///   All reads and writes go through Store (Redis).
///   No std::fs calls exist in this binary.
///   Prompt templates are the sole exception: they are loaded once at
///   startup from a container-mounted read-only volume and validated
///   before the worker loop starts.  Paths are not user-controlled.
use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use some_transport::nats::{JetStreamConfig, JetStreamPublisher, NatsTransport};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tokio::time::Duration;
use tracing::info;
use tracing_subscriber::EnvFilter;

use tabsched_pipeline::{
	llm::{LlmBackend, LocalLlm, LocalLlmConfig, MarkdownDump, MarkdownDumpConfig},
	runtime::{worker, Store, WorkerCtx},
	stages::embed::EmbedProvider,
};

// ── CLI ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, ValueEnum)]
enum LlmMode {
	Local,
	Dump,
}

#[derive(Debug, Clone, ValueEnum)]
enum EmbedMode {
	Ollama,
	Openai,
}

#[derive(Debug, Parser)]
#[command(name = "tabsched-pipeline", about = "Fault-tolerant pipeline daemon")]
struct Args {
	/// Directory containing derive-edges.md and derive-tracks.md.
	/// Must exist at startup — validated before workers start.
	/// Should be a read-only container volume mount.
	#[arg(long, env = "PIPELINE_PROMPTS_DIR")]
	prompts_dir: PathBuf,

	#[arg(long, value_enum, default_value = "local", env = "LLM_MODE")]
	llm_mode: LlmMode,

	#[arg(long, value_enum, default_value = "ollama", env = "EMBED_PROVIDER")]
	embed_provider: EmbedMode,

	#[arg(long, default_value = "http://localhost:11434", env = "OLLAMA_HOST")]
	ollama_host: String,

	#[arg(long, default_value = "llama3", env = "LLM_MODEL")]
	llm_model: String,

	#[arg(long, env = "OPENAI_API_KEY")]
	openai_api_key: Option<String>,

	#[arg(long, default_value = "text-embedding-3-small", env = "EMBED_MODEL")]
	embed_model: String,

	#[arg(long, default_value = "0.65", env = "SIMILARITY_THRESHOLD")]
	similarity_threshold: f32,

	#[arg(long, default_value = "20", env = "WINDOW_SIZE")]
	window_size: u32,

	/// Redis URL for the artifact store.
	#[arg(long, default_value = "redis://127.0.0.1:6379", env = "REDIS_URL")]
	redis_url: String,

	/// NATS server URL for JetStream job queue.
	#[arg(long, default_value = "nats://127.0.0.1:4222", env = "NATS_URL")]
	nats_url: String,

	/// Number of concurrent worker tasks.
	/// Each worker holds one JetStream message at a time.
	/// On CPU with local Ollama, 2 is the recommended ceiling.
	#[arg(long, default_value = "2", env = "PIPELINE_WORKERS")]
	workers: usize,

	/// JetStream ack_wait in seconds.  Must exceed worst-case stage duration.
	#[arg(long, default_value = "600", env = "PIPELINE_ACK_WAIT_SECS")]
	ack_wait_secs: u64,

	/// JetStream max_deliver before server terminates → DLQ.
	#[arg(long, default_value = "5", env = "PIPELINE_MAX_DELIVER")]
	max_deliver: i64,

	/// JetStream max_payload_size that triggers a nak
	#[arg(long, default_value = "536870912", env = "NATS_MAX_STREAM_BYTES")]
	nats_max_bytes: i64,

	#[arg(long, default_value = "8388608", env = "NATS_MAX_MSG_BYTES")]
	nats_max_msg_size: i32,

	/// Dump-mode prompt directory (--llm-mode=dump only).
	#[arg(long, default_value = "pipeline-prompts", env = "PROMPTS_OUT_DIR")]
	prompts_out_dir: PathBuf,
}

// ── Pre-flight checks ─────────────────────────────────────────────────────

/// Load prompt templates from the container-mounted prompts directory.
/// Fails fast if either file is missing — workers must not start without them.
/// This is the only std::fs call in the binary; paths are not user-controlled.
fn load_prompt_templates(dir: &PathBuf) -> Result<(String, String)> {
	let edge_path = dir.join("derive-edges.md");
	let track_path = dir.join("derive-tracks.md");

	let edge_template = std::fs::read_to_string(&edge_path).with_context(|| format!("missing prompt template: {:?}", edge_path))?;
	let track_template = std::fs::read_to_string(&track_path).with_context(|| format!("missing prompt template: {:?}", track_path))?;

	if edge_template.trim().is_empty() {
		anyhow::bail!("derive-edges.md is empty");
	}
	if track_template.trim().is_empty() {
		anyhow::bail!("derive-tracks.md is empty");
	}

	info!("Prompt templates loaded from {:?}", dir);
	Ok((edge_template, track_template))
}

// ── Main ───────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
	// Load .env file into the environment before parsing arguments.
	// If the file is missing, we ignore the error and fall back to
	// actual environment variables or defaults.
	let _ = dotenvy::dotenv();

	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
		.with_writer(std::io::stderr)
		.init();

	let args = Args::parse();

	// ── Pre-flight ────────────────────────────────────────────────────────

	// 1. Load prompt templates — fail fast before spawning workers.
	let (edge_template, track_template) = load_prompt_templates(&args.prompts_dir)?;

	// 2. Validate worker count.
	if args.workers == 0 {
		anyhow::bail!("PIPELINE_WORKERS must be >= 1");
	}

	// 3. Build embed provider.
	let embed_provider = match args.embed_provider {
		EmbedMode::Ollama => EmbedProvider::Ollama {
			host: args.ollama_host.clone(),
			model: "nomic-embed-text".into(),
		},
		EmbedMode::Openai => EmbedProvider::OpenAI {
			api_key: args.openai_api_key.clone().context("OPENAI_API_KEY required for embed-provider=openai")?,
			base_url: "https://api.openai.com/v1".into(),
			model: args.embed_model.clone(),
		},
	};

	// 4. Build LLM backend.
	let llm: Arc<dyn LlmBackend> = match args.llm_mode {
		LlmMode::Local => Arc::new(LocalLlm::new(LocalLlmConfig {
			base_url: format!("{}/v1", args.ollama_host),
			model: args.llm_model.clone(),
			api_key: args.openai_api_key.clone(),
			max_tokens: 4096,
			temperature: 0.1,
		})),
		LlmMode::Dump => Arc::new(MarkdownDump::new(MarkdownDumpConfig {
			out_dir: args.prompts_out_dir.clone(),
			poll_interval_secs: 5,
			// Dump mode requires an explicit timeout in daemon context —
			// an infinite wait would stall the worker and exhaust ack_wait,
			// causing repeated redelivery.  10 minutes is generous for manual paste.
			timeout_secs: 600,
		})),
	};

	// 5. Connect Redis store (shared across workers via ConnectionManager clone).
	let store = Store::connect(&args.redis_url).await?;
	info!("Redis connected: {}", args.redis_url);

	// 6. Connect NATS publisher (for DLQ publishes).
	let transport = NatsTransport::connect_pooled(&args.nats_url).await?;
	let nats_client = transport.client().clone();
	let publisher = Arc::new(JetStreamPublisher::from_client(nats_client.clone()));
	info!("NATS publisher connected: {}", args.nats_url);

	// ── Build shared context ──────────────────────────────────────────────
	let ctx = WorkerCtx {
		http: reqwest::Client::builder()
			// No global timeout — per-stage timeouts are enforced inside each stage.
			.build()?,
		embed_provider: Arc::new(embed_provider),
		llm,
		store,
		publisher,
		transport: transport.clone(),
		edge_template: Arc::new(edge_template),
		track_template: Arc::new(track_template),
		similarity_threshold: args.similarity_threshold,
		window_size: args.window_size,
	};

	let js_config = JetStreamConfig {
		nats_url: args.nats_url.clone(),
		consumer_name: "pipeline-worker".into(),
		ack_wait: Duration::from_secs(args.ack_wait_secs),
		max_deliver: args.max_deliver,
		max_bytes: args.nats_max_bytes,
		max_message_size: args.nats_max_msg_size,
		fetch_batch: 1,
	};

	// ── Shutdown signal ───────────────────────────────────────────────────
	let shutdown = Arc::new(tokio::sync::Notify::new());

	// Spawn shutdown listener.
	let shutdown_tx = shutdown.clone();
	tokio::spawn(async move {
		signal::ctrl_c().await.ok();
		info!("SIGINT received — notifying workers");
		// Notify all workers (notify_waiters broadcasts).
		shutdown_tx.notify_waiters();
	});

	// ── Spawn workers ─────────────────────────────────────────────────────
	info!(workers = args.workers, "spawning worker tasks");

	let mut handles = Vec::with_capacity(args.workers);
	for id in 0..args.workers {
		let ctx = ctx.clone();
		let config = js_config.clone();
		let sd = shutdown.clone();
		handles.push(tokio::spawn(async move {
			worker(id, ctx, config, sd).await;
		}));
	}

	// Wait for all workers to exit cleanly.
	for h in handles {
		h.await.ok();
	}

	info!("all workers stopped — daemon exiting");
	Ok(())
}

use anyhow::Result;
use std::time::Instant;
use tracing::info;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Load Whisper model from disk
pub fn load_model(model_path: &str, threads: i32) -> Result<WhisperContext> {
	info!("🔄 Loading Whisper model from {}...", model_path);
	let start = Instant::now();

	let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())?;

	let load_time = start.elapsed();
	info!(load_time_ms = load_time.as_millis(), threads, "✅ Whisper model loaded");

	Ok(ctx)
}

/// Create Whisper transcription parameters
pub fn create_params(threads: i32) -> FullParams<'static, 'static> {
	let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
	params.set_translate(false);
	params.set_print_special(false);
	params.set_print_progress(false);
	params.set_print_realtime(false);
	params.set_print_timestamps(false);
	params.set_n_threads(threads);

	info!(whisper_threads = threads, "🔧 Whisper configured");
	params
}

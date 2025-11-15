use anyhow::Result;
use opentelemetry::KeyValue;
use some_transport::{NatsTransport, Transport};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use ws_events::events::{Event, UnifiedEvent};

use crate::observability::TranscriberMetrics;
use crate::state::TranscriberState;

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

/// Transcribe audio and publish results
/// Spawns work in background - does not return a handle
/// On shutdown, blocking Whisper threads will be abandoned and cleaned up by OS
pub fn transcribe_and_publish(
	ctx: Arc<WhisperContext>,
	params: FullParams<'static, 'static>,
	transport: NatsTransport<UnifiedEvent>,
	audio: Vec<f32>,
	state: Arc<TranscriberState>,
	metrics: TranscriberMetrics,
	_permit: tokio::sync::OwnedSemaphorePermit,
	cancellation_token: CancellationToken,
) {
	tokio::task::spawn(async move {
		// Spawn the blocking transcription work
		// Note: This cannot be cancelled mid-execution due to FFI limitations
		// On shutdown, this thread will be abandoned and cleaned up by the OS
		let transcription_task = tokio::task::spawn_blocking({
			let ctx = ctx.clone();
			let state = state.clone();
			let metrics = metrics.clone();

			move || {
				let result = transcribe_audio(&ctx, params, &audio, &state, &metrics);

				match result {
					Ok(segments) => Some(segments),
					Err(e) => {
						error!(error = %e, "Transcription pipeline failed");
						state.set_transcribing(false);
						None
					}
				}
			}
		});

		// Wait for transcription to complete OR cancellation
		let segments = tokio::select! {
			_ = cancellation_token.cancelled() => {
				info!("🛑 Transcription cancelled - abandoning blocking work (OS will clean up)");
				state.set_transcribing(false);
				return;
			}
			result = transcription_task => {
				state.set_transcribing(false);

				match result {
					Ok(Some(segments)) => segments,
					Ok(None) => return,
					Err(e) => {
						error!(error = %e, "Transcription task panicked");
						return;
					}
				}
			}
		};

		// Publish segments if we got results and weren't cancelled
		if !cancellation_token.is_cancelled() {
			publish_segments(segments, transport, &state, &metrics, cancellation_token).await;
		} else {
			info!("🛑 Skipping publish due to cancellation");
		}
	});
}

fn transcribe_audio(ctx: &WhisperContext, params: FullParams<'static, 'static>, audio: &[f32], state: &TranscriberState, metrics: &TranscriberMetrics) -> Result<Vec<String>> {
	let audio_duration_secs = audio.len() as f64 / 16000.0;

	info!(
		audio_samples = audio.len(),
		duration_secs = format!("{:.2}", audio_duration_secs),
		"🎬 [TRANSCRIBE START] Beginning transcription..."
	);

	let transcribe_start = Instant::now();

	// Create Whisper state
	info!("🔧 [STEP 1/3] Creating Whisper state...");
	let mut whisper_state = ctx.create_state().map_err(|e| {
		state.transcriptions_failed.fetch_add(1, Ordering::Relaxed);
		metrics.transcriptions_failed.add(1, &[KeyValue::new("error", "state_creation_failed")]);
		anyhow::anyhow!("Failed to create Whisper state: {}", e)
	})?;

	info!("✅ [STEP 1/3] Whisper state created");

	// Run transcription
	// NOTE: This is a blocking FFI call that cannot be interrupted
	// If shutdown happens during this call, the thread will be abandoned
	// and cleaned up by the OS when the process exits
	info!("🧠 [STEP 2/3] Running Whisper model...");
	whisper_state.full(params, audio).map_err(|e| {
		state.transcriptions_failed.fetch_add(1, Ordering::Relaxed);
		metrics.transcriptions_failed.add(1, &[KeyValue::new("error", e.to_string())]);
		anyhow::anyhow!("Transcription failed: {}", e)
	})?;

	let transcribe_latency = transcribe_start.elapsed().as_secs_f64() * 1000.0;
	let realtime_factor = transcribe_latency / 1000.0 / audio_duration_secs;

	info!(
		transcribe_latency_ms = format!("{:.0}", transcribe_latency),
		realtime_factor = format!("{:.2}x", realtime_factor),
		"✅ [STEP 2/3] Transcription completed"
	);

	metrics.transcription_latency.record(transcribe_latency, &[]);
	state.transcriptions_completed.fetch_add(1, Ordering::Relaxed);
	metrics.transcriptions_completed.add(1, &[]);

	// Extract segments
	info!("📋 [STEP 3/3] Extracting segments...");
	let num_segments = whisper_state.full_n_segments();

	if num_segments == 0 {
		warn!("⚠️ No segments extracted - audio may be silence");
		return Ok(Vec::new());
	}

	info!(num_segments, "📋 Extracted {} segment(s)", num_segments);

	let mut segments = Vec::new();
	for i in 0..num_segments {
		if let Some(segment) = whisper_state.get_segment(i) {
			if let Ok(text) = segment.to_str() {
				let trimmed = text.trim();
				if !trimmed.is_empty() {
					segments.push(trimmed.to_string());
				}
			}
		}
	}

	Ok(segments)
}

async fn publish_segments(
	segments: Vec<String>,
	transport: NatsTransport<UnifiedEvent>,
	state: &Arc<TranscriberState>,
	metrics: &TranscriberMetrics,
	cancellation_token: CancellationToken,
) {
	info!(segment_count = segments.len(), "📡 Publishing {} segment(s)...", segments.len());

	for (i, text) in segments.iter().enumerate() {
		// Check if cancelled before each publish
		if cancellation_token.is_cancelled() {
			info!("🛑 Publishing cancelled at segment {}/{}", i + 1, segments.len());
			break;
		}

		let emoji = match text.len() {
			0..=19 => "💬",
			20..=49 => "📝",
			_ => "📄",
		};

		info!(
				segment = i + 1,
				total = segments.len(),
				text_length = text.len(),
				text = %text,
				"{} [SEGMENT {}/{}] \"{}\"",
				emoji,
				i + 1,
				segments.len(),
				text
		);

		let event = Event::Subtitle {
			text: text.clone(),
			timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
			confidence: None,
		};

		if let Some(unified) = Option::<UnifiedEvent>::from(event) {
			let subject = unified.subject().unwrap_or_else(|| "audio.subtitle".to_string());

			let transport_clone = transport.clone();
			let metrics_clone = metrics.clone();
			let state_clone = Arc::clone(state);
			let cancellation_token_clone = cancellation_token.clone();

			// Spawn publish task - these will exit gracefully on cancellation
			tokio::spawn(async move {
				tokio::select! {
					_ = cancellation_token_clone.cancelled() => {
						debug!("🛑 Segment publish cancelled");
					}
					result = transport_clone.send_to_subject(&subject, unified) => {
						match result {
							Ok(_) => {
								state_clone.subtitles_published.fetch_add(1, Ordering::Relaxed);
								metrics_clone.subtitles_published.add(1, &[]);
								debug!("✅ Segment published to NATS");
							}
							Err(e) => {
								error!(error = %e, "❌ Failed to publish subtitle");
							}
						}
					}
				}
			});
		}
	}

	info!(published = segments.len(), "✨ Publishing complete - {} subtitle(s) sent", segments.len());
}

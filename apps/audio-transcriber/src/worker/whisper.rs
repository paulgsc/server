use anyhow::Result;
use opentelemetry::KeyValue;
use some_transport::{NatsTransport, Transport};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use whisper_rs::{FullParams, WhisperContext};
use ws_events::events::{Event, UnifiedEvent};

use super::queue::TranscriptionJob;
use crate::observability::TranscriberMetrics;
use crate::state::TranscriberState;

/// Start the blocking Whisper worker thread
///
/// This spawns exactly ONE blocking worker that processes jobs sequentially.
/// The worker runs in a dedicated thread and cannot be cancelled mid-transcription.
///
/// On shutdown:
/// - Cancellation token signals worker to stop accepting new jobs
/// - Worker exits gracefully after current job completes
/// - If worker is mid-job during shutdown, the thread is abandoned and OS cleans it up
pub fn start_whisper_worker(
	mut rx: mpsc::Receiver<TranscriptionJob>,
	whisper_ctx: Arc<WhisperContext>,
	params: FullParams<'static, 'static>,
	transport: NatsTransport<UnifiedEvent>,
	state: Arc<TranscriberState>,
	metrics: TranscriberMetrics,
	cancellation_token: CancellationToken,
) {
	info!("🏭 Starting Whisper worker thread");

	// Spawn ONE blocking worker - this is a CPU drainpipe
	tokio::task::spawn_blocking(move || whisper_worker_loop(&mut rx, &whisper_ctx, params, transport, state, metrics, cancellation_token));
}

/// Main worker loop - runs in blocking context
///
/// This is NOT async. It is a dedicated CPU loop that:
/// - Never touches async primitives directly
/// - Never awaits
/// - Never spawns more workers
fn whisper_worker_loop(
	rx: &mut mpsc::Receiver<TranscriptionJob>,
	whisper_ctx: &WhisperContext,
	params: FullParams<'static, 'static>,
	transport: NatsTransport<UnifiedEvent>,
	state: Arc<TranscriberState>,
	metrics: TranscriberMetrics,
	cancellation_token: CancellationToken,
) {
	info!("🔄 Worker loop started, waiting for jobs...");

	loop {
		// Check for shutdown before blocking on receive
		if cancellation_token.is_cancelled() {
			info!("🛑 Worker shutting down (cancellation requested)");
			break;
		}

		// Blocking receive - waits for next job
		let job = match rx.blocking_recv() {
			Some(job) => job,
			None => {
				info!("🛑 Worker shutting down (queue sender dropped)");
				break;
			}
		};

		// Log queue latency
		let queue_latency_ms = job.queue_latency().as_millis() as f64;
		metrics.transcription_queue_latency.record(queue_latency_ms, &[]);

		info!(
			seq = job.seq,
			queue_latency_ms = format!("{:.0}", queue_latency_ms),
			audio_duration_secs = format!("{:.2}", job.audio_duration_secs()),
			"📥 Processing job from queue"
		);

		// Set worker busy
		state.set_transcribing(true);

		// Process the job (BLOCKING - cannot be cancelled)
		let job_start = Instant::now();

		match process_transcription_job(job, whisper_ctx, &params, &state, &metrics) {
			Ok(segments) => {
				let processing_latency_ms = job_start.elapsed().as_millis() as f64;
				metrics.transcription_processing_latency.record(processing_latency_ms, &[]);

				// Publish results (async boundary)
				publish_segments_sync(segments, &transport, &state, &metrics);
			}
			Err(e) => {
				error!(error = %e, "❌ Transcription job failed");
			}
		}

		// Clear worker busy
		state.set_transcribing(false);
	}

	info!("✅ Worker thread exiting");
}

/// Process a single transcription job (blocking)
fn process_transcription_job(
	job: TranscriptionJob,
	whisper_ctx: &WhisperContext,
	params: &FullParams<'static, 'static>,
	state: &TranscriberState,
	metrics: &TranscriberMetrics,
) -> Result<Vec<String>> {
	let audio_duration_secs = job.audio_duration_secs();

	info!(
		seq = job.seq,
		audio_samples = job.audio.len(),
		duration_secs = format!("{:.2}", audio_duration_secs),
		"🎬 [TRANSCRIBE START] Beginning transcription..."
	);

	let transcribe_start = Instant::now();

	// Create Whisper state
	info!("🔧 [STEP 1/3] Creating Whisper state...");
	let mut whisper_state = whisper_ctx.create_state().map_err(|e| {
		state.transcriptions_failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		metrics.transcriptions_failed.add(1, &[KeyValue::new("error", "state_creation_failed")]);
		anyhow::anyhow!("Failed to create Whisper state: {}", e)
	})?;

	info!("✅ [STEP 1/3] Whisper state created");

	// Run transcription (BLOCKING FFI - cannot be interrupted)
	info!("🧠 [STEP 2/3] Running Whisper model...");
	whisper_state.full(params.clone(), &job.audio).map_err(|e| {
		state.transcriptions_failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		metrics.transcriptions_failed.add(1, &[KeyValue::new("error", e.to_string())]);
		anyhow::anyhow!("Transcription failed: {}", e)
	})?;

	let transcribe_latency = transcribe_start.elapsed().as_secs_f64() * 1000.0;
	let realtime_factor = transcribe_latency / 1000.0 / audio_duration_secs;

	info!(
		seq = job.seq,
		transcribe_latency_ms = format!("{:.0}", transcribe_latency),
		realtime_factor = format!("{:.2}x", realtime_factor),
		"✅ [STEP 2/3] Transcription completed"
	);

	metrics.transcription_latency.record(transcribe_latency, &[]);
	state.transcriptions_completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

/// Publish segments from blocking context
///
/// This uses a blocking runtime handle to publish async
fn publish_segments_sync(segments: Vec<String>, transport: &NatsTransport<UnifiedEvent>, state: &Arc<TranscriberState>, metrics: &TranscriberMetrics) {
	if segments.is_empty() {
		return;
	}

	info!(segment_count = segments.len(), "📡 Publishing {} segment(s)...", segments.len());

	// Get or create runtime handle for async operations from blocking context
	let handle = tokio::runtime::Handle::current();

	for (i, text) in segments.iter().enumerate() {
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

			// Use handle to spawn async task from blocking context
			handle.spawn(async move {
				match transport_clone.send_to_subject(&subject, unified).await {
					Ok(_) => {
						state_clone.subtitles_published.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
						metrics_clone.subtitles_published.add(1, &[]);
						debug!("✅ Segment published to NATS");
					}
					Err(e) => {
						error!(error = %e, "❌ Failed to publish subtitle");
					}
				}
			});
		}
	}

	info!(published = segments.len(), "✨ Publishing complete - {} subtitle(s) sent", segments.len());
}

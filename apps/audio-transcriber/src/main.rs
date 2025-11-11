mod observability;

use anyhow::Result;
use observability::{init_observability, Heartbeat, TranscriberMetrics};
use opentelemetry::KeyValue;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, instrument, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use some_transport::{NatsTransport, Transport};
use ws_events::events::{Event, UnifiedEvent};

// Global state for observability
struct TranscriberState {
	chunks_received: AtomicU64,
	bytes_received: AtomicU64,
	samples_processed: AtomicU64,
	transcriptions_completed: AtomicU64,
	transcriptions_failed: AtomicU64,
	subtitles_published: AtomicU64,
	buffer_size: AtomicUsize,
	current_sample_rate: AtomicU64,
	is_transcribing: AtomicBool, // Track if transcription is in progress
}

impl TranscriberState {
	fn new() -> Arc<Self> {
		Arc::new(Self {
			chunks_received: AtomicU64::new(0),
			bytes_received: AtomicU64::new(0),
			samples_processed: AtomicU64::new(0),
			transcriptions_completed: AtomicU64::new(0),
			transcriptions_failed: AtomicU64::new(0),
			subtitles_published: AtomicU64::new(0),
			buffer_size: AtomicUsize::new(0),
			current_sample_rate: AtomicU64::new(0),
			is_transcribing: AtomicBool::new(false),
		})
	}
}

#[tokio::main]
#[instrument]
async fn main() -> Result<()> {
	// Initialize OpenTelemetry
	let service_name = std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "transcriber".to_string());
	let (_meter_provider, metrics) = init_observability(&service_name)?;

	info!("🎯 Starting CPU-optimized transcriber...");

	// Initialize state
	let state = TranscriberState::new();

	// Register gauge callbacks
	let state_clone = state.clone();
	let meter = opentelemetry::global::meter("transcriber");
	let _buffer_size_registration = meter
		.u64_observable_gauge("transcriber.buffer.size")
		.with_callback(move |observer| {
			observer.observe(state_clone.buffer_size.load(Ordering::Relaxed) as u64, &[]);
		})
		.init();

	let state_clone = state.clone();
	let _sample_rate_registration = meter
		.u64_observable_gauge("transcriber.sample_rate")
		.with_callback(move |observer| {
			observer.observe(state_clone.current_sample_rate.load(Ordering::Relaxed), &[]);
		})
		.init();

	let _heartbeat_registration = meter
		.u64_observable_gauge("transcriber.heartbeat")
		.with_callback(move |observer| {
			let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
			observer.observe(timestamp, &[]);
		})
		.init();

	// Connect using your transport
	let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

	let transport = connect_to_nats(&nats_url).await?;
	info!("✅ Connected to NATS at {}", nats_url);

	// Load Whisper model
	let model_path = std::env::var("WHISPER_MODEL").unwrap_or_else(|_| "/models/ggml-base.en-q5_1.bin".to_string());
	let ctx = load_whisper_model(&model_path)?;
	let ctx = Arc::new(ctx);

	// Setup Whisper parameters
	let params = setup_whisper_params();

	// Get configuration
	let target_sample_rate = 16000_u32;
	let buffer_duration_secs = std::env::var("BUFFER_DURATION").ok().and_then(|s| s.parse().ok()).unwrap_or(3); // Default 3 seconds for CPU efficiency
	let buffer_size = target_sample_rate as usize * buffer_duration_secs;

	// Semaphore to limit concurrent transcriptions (prevent CPU overload)
	let transcription_semaphore = Arc::new(Semaphore::new(1)); // Only 1 transcription at a time

	// Subscribe to audio chunks
	let mut receiver = transport.subscribe_to_subject("audio.chunk").await;
	info!("🎧 Subscribed to 'audio.chunk', waiting for audio...");

	let mut audio_buffer: Vec<f32> = Vec::with_capacity(buffer_size * 2);
	let mut known_sample_rate: Option<u32> = None;
	let mut known_channels: Option<u32> = None;

	// Heartbeat tracker
	let mut heartbeat = Heartbeat::new(30);

	info!(
		target_sample_rate,
		buffer_duration_secs,
		buffer_size,
		whisper_threads = std::env::var("WHISPER_THREADS").unwrap_or_else(|_| "2".to_string()),
		"📊 CPU-optimized configuration"
	);

	loop {
		// Heartbeat logging
		if heartbeat.maybe_log(
			state.chunks_received.load(Ordering::Relaxed),
			state.bytes_received.load(Ordering::Relaxed),
			state.samples_processed.load(Ordering::Relaxed),
			audio_buffer.len(),
			state.transcriptions_completed.load(Ordering::Relaxed),
		) {
			// Update buffer size gauge
			state.buffer_size.store(audio_buffer.len(), Ordering::Relaxed);

			// Log if transcription is in progress
			if state.is_transcribing.load(Ordering::Relaxed) {
				info!("⚙️  Transcription in progress (CPU busy)");
			}
		}

		// Receive audio chunk with timeout to allow heartbeat checks
		let chunk_start = Instant::now();

		let unified = match tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await {
			Ok(Ok(event)) => event,
			Ok(Err(e)) => {
				error!(error = %e, "Failed to receive event");
				metrics.chunks_dropped.add(1, &[KeyValue::new("reason", "recv_error")]);
				continue;
			}
			Err(_) => {
				// Timeout - continue to allow heartbeat and other checks
				continue;
			}
		};

		// Convert to Event
		let event: Event = match Result::<Event, String>::from(unified) {
			Ok(e) => e,
			Err(e) => {
				error!(error = %e, "Failed to convert UnifiedEvent");
				metrics.chunks_dropped.add(1, &[KeyValue::new("reason", "conversion_error")]);
				continue;
			}
		};

		// Process audio chunk
		if let Event::AudioChunk { sample_rate, channels, samples } = event {
			let chunk_bytes = samples.len() * std::mem::size_of::<f32>();

			// Update metrics
			state.chunks_received.fetch_add(1, Ordering::Relaxed);
			state.bytes_received.fetch_add(chunk_bytes as u64, Ordering::Relaxed);
			metrics.chunks_received.add(1, &[]);
			metrics.bytes_received.add(chunk_bytes as u64, &[]);

			// Update known format
			if sample_rate > 0 {
				if known_sample_rate.is_none() {
					info!(sample_rate, channels, "📊 Audio format detected");
					state.current_sample_rate.store(sample_rate as u64, Ordering::Relaxed);
				}
				known_sample_rate = Some(sample_rate);
				known_channels = Some(channels);
			}

			let actual_sample_rate = known_sample_rate.unwrap_or(sample_rate);
			let actual_channels = known_channels.unwrap_or(channels);

			// Process audio
			let processed_samples = process_audio_chunk(samples, actual_sample_rate, actual_channels, target_sample_rate, &mut audio_buffer, &metrics).await;

			state.samples_processed.fetch_add(processed_samples as u64, Ordering::Relaxed);
			metrics.samples_processed.add(processed_samples as u64, &[]);

			// Record chunk processing latency
			let chunk_latency = chunk_start.elapsed().as_secs_f64() * 1000.0;
			metrics.chunk_processing_latency.record(chunk_latency, &[]);

			// Log periodic stats
			let chunks = state.chunks_received.load(Ordering::Relaxed);
			if chunks % 50 == 0 {
				debug!(
					chunks,
					buffer_size = audio_buffer.len(),
					is_transcribing = state.is_transcribing.load(Ordering::Relaxed),
					"📦 Processing stats"
				);
			}

			// Transcribe when buffer is full AND no transcription in progress
			if audio_buffer.len() >= buffer_size {
				// Try to acquire semaphore (non-blocking)
				if let Ok(permit) = transcription_semaphore.clone().try_acquire_owned() {
					// Record buffer fill time

					let transcription_audio = audio_buffer.clone();
					audio_buffer.clear();

					// Update buffer size
					state.buffer_size.store(0, Ordering::Relaxed);

					// Mark transcription as in progress
					state.is_transcribing.store(true, Ordering::Relaxed);

					info!(audio_samples = transcription_audio.len(), "🎤 Starting transcription (CPU will be busy)");

					// Transcribe in blocking task
					transcribe_and_publish(
						ctx.clone(),
						params.clone(),
						transport.clone(),
						transcription_audio,
						state.clone(),
						metrics.clone(),
						permit, // Pass permit to release when done
					)
					.await;

					// Reset buffer fill timer
				} else {
					// Transcription still in progress - keep buffering
					debug!(buffer_size = audio_buffer.len(), "⏳ Transcription in progress, buffering audio");
				}
			}

			// Update buffer size gauge
			state.buffer_size.store(audio_buffer.len(), Ordering::Relaxed);
		}
	}
}

#[instrument(skip(nats_url))]
async fn connect_to_nats(nats_url: &str) -> Result<NatsTransport<UnifiedEvent>> {
	let transport = NatsTransport::<UnifiedEvent>::connect_pooled(nats_url).await?;
	Ok(transport)
}

#[instrument]
fn load_whisper_model(model_path: &str) -> Result<WhisperContext> {
	info!("🔄 Loading Whisper model from {}...", model_path);
	let start = Instant::now();
	let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())?;
	let load_time = start.elapsed();
	info!(load_time_ms = load_time.as_millis(), "✅ Whisper model loaded");
	Ok(ctx)
}

#[instrument]
fn setup_whisper_params() -> FullParams<'static, 'static> {
	let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
	params.set_translate(false);
	params.set_print_special(false);
	params.set_print_progress(false);
	params.set_print_realtime(false);
	params.set_print_timestamps(false);

	// Get thread count from env (default: 2 for CPU efficiency)
	let threads = std::env::var("WHISPER_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(2);

	params.set_n_threads(threads);
	info!(whisper_threads = threads, "🔧 Whisper configured");
	params
}

#[instrument(skip(samples, audio_buffer, metrics))]
async fn process_audio_chunk(samples: Vec<f32>, sample_rate: u32, channels: u32, target_sample_rate: u32, audio_buffer: &mut Vec<f32>, metrics: &TranscriberMetrics) -> usize {
	// Convert to mono if stereo
	let mono_samples: Vec<f32> = if channels == 2 {
		samples.chunks(2).map(|stereo| (stereo[0] + stereo[1]) / 2.0).collect()
	} else {
		samples
	};

	// Resample if needed
	let resampled = if sample_rate != target_sample_rate {
		let resample_start = Instant::now();
		let resampled = resample_simple(&mono_samples, sample_rate, target_sample_rate);
		let resample_latency = resample_start.elapsed().as_secs_f64() * 1000.0;
		metrics.resampling_latency.record(resample_latency, &[]);
		resampled
	} else {
		mono_samples
	};

	let sample_count = resampled.len();
	audio_buffer.extend(resampled);
	sample_count
}

#[instrument(skip(ctx, params, transport, audio, state, metrics, _permit))]
async fn transcribe_and_publish(
	ctx: Arc<WhisperContext>,
	params: FullParams<'static, 'static>,
	transport: NatsTransport<UnifiedEvent>,
	audio: Vec<f32>,
	state: Arc<TranscriberState>,
	metrics: TranscriberMetrics,
	_permit: tokio::sync::OwnedSemaphorePermit, // Holds the lock
) {
	tokio::task::spawn_blocking(move || {
		let transcribe_start = Instant::now();

		if let Ok(mut whisper_state) = ctx.create_state() {
			match whisper_state.full(params, &audio) {
				Ok(_) => {
					let transcribe_latency = transcribe_start.elapsed().as_secs_f64() * 1000.0;
					metrics.transcription_latency.record(transcribe_latency, &[]);

					state.transcriptions_completed.fetch_add(1, Ordering::Relaxed);
					metrics.transcriptions_completed.add(1, &[]);

					let num_segments = whisper_state.full_n_segments();
					info!(num_segments, transcribe_latency_ms = transcribe_latency as u64, "✅ Transcription completed");

					for i in 0..num_segments {
						if let Some(segment) = whisper_state.get_segment(i) {
							// ✅ Updated for new whisper-rs API
							if let Ok(text) = segment.to_str() {
								let trimmed = text.trim();
								if !trimmed.is_empty() {
									info!(text = %trimmed, segment = i, "📝 Subtitle");

									// Create subtitle event
									let event = Event::Subtitle {
										text: trimmed.to_string(),
										timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
										confidence: None,
									};

									// Publish to NATS
									if let Some(unified) = Option::<UnifiedEvent>::from(event) {
										let subject = unified.subject().unwrap_or_else(|| "audio.subtitle".to_string());

										let transport_clone = transport.clone();
										let metrics_clone = metrics.clone();
										let state_clone = state.clone();

										tokio::spawn(async move {
											match transport_clone.send_to_subject(&subject, unified).await {
												Ok(_) => {
													state_clone.subtitles_published.fetch_add(1, Ordering::Relaxed);
													metrics_clone.subtitles_published.add(1, &[]);
												}
												Err(e) => {
													error!(error = %e, "Failed to publish subtitle");
												}
											}
										});
									}
								}
							} else {
								warn!(segment = i, "⚠️ Failed to decode segment text");
							}
						}
					}
				}
				Err(e) => {
					error!(error = %e, "Transcription failed");
					state.transcriptions_failed.fetch_add(1, Ordering::Relaxed);
					metrics.transcriptions_failed.add(1, &[KeyValue::new("error", e.to_string())]);
				}
			}
		} else {
			error!("Failed to create Whisper state");
			state.transcriptions_failed.fetch_add(1, Ordering::Relaxed);
			metrics.transcriptions_failed.add(1, &[KeyValue::new("error", "state_creation_failed")]);
		}

		// Mark transcription as complete (permit is dropped here)
		state.is_transcribing.store(false, Ordering::Relaxed);
		info!("✅ CPU available for next transcription");
	});
}

fn resample_simple(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
	if from_rate == to_rate {
		return samples.to_vec();
	}

	let ratio = from_rate as f32 / to_rate as f32;
	let output_len = (samples.len() as f32 / ratio) as usize;

	(0..output_len)
		.map(|i| {
			let src_idx = (i as f32 * ratio) as usize;
			samples.get(src_idx).copied().unwrap_or(0.0)
		})
		.collect()
}

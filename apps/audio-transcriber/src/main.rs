mod audio;
mod config;
mod observability;
mod state;
mod transcription;
mod vad;
mod worker;

use anyhow::Result;
use clap::Parser;
use some_transport::{NatsReceiver, NatsTransport, TransportReceiver};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_events::events::{AudioChunkMessage, EventType, UnifiedEvent};

use config::Config;
use state::TranscriberState;
use worker::{TranscriptionJob, TranscriptionQueue, TRANSCRIPTION_QUEUE_CAPACITY};

const NATS_MAX_RETRIES: u32 = 5;
const NATS_INITIAL_BACKOFF_MS: u64 = 500;
const SHUTDOWN_GRACE_PERIOD_MS: u64 = 200;

#[tokio::main]
async fn main() -> Result<()> {
	// Load environment variables
	dotenvy::dotenv().ok();

	// Parse CLI arguments
	let config = Config::parse();
	config.validate().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

	// Initialize observability (vendor has built-in retries, we fallback to local-only)
	let (_meter_provider, metrics) = init_observability_with_fallback(&config).await;

	info!(
			service = %config.service_name,
			whisper_model = %config.whisper_model_path,
			vad_enabled = config.vad_enabled,
			vad_threshold = config.vad_speech_threshold,
			"🎯 Starting transcriber service"
	);

	// Initialize state
	let state = TranscriberState::new();
	state.register_gauges()?;

	// Connect to transport with retry
	let transport = connect_with_retry(&config).await?;

	// Load Whisper model
	let whisper_ctx = transcription::load_model(&config.whisper_model_path, config.whisper_threads)?;

	// Create transcription queue
	let mut queue = TranscriptionQueue::new(TRANSCRIPTION_QUEUE_CAPACITY);
	let queue_tx = queue.sender();
	let queue_rx = queue.take_receiver().expect("Queue receiver should be available");

	info!(queue_capacity = TRANSCRIPTION_QUEUE_CAPACITY, "📦 Transcription queue created");

	// Create cancellation token for cooperative shutdown
	let cancellation_token = CancellationToken::new();

	// Start Whisper worker thread
	let params = transcription::create_params(config.whisper_threads);
	worker::start_whisper_worker(
		queue_rx,
		Arc::new(whisper_ctx),
		params,
		transport.clone(),
		state.clone(),
		metrics.clone(),
		cancellation_token.clone(),
	);

	// Start transcription loop
	let transcriber = Transcriber {
		config,
		state,
		metrics,
		transport,
		queue_tx,
		cancellation_token: cancellation_token.clone(),
	};

	// Run with graceful shutdown
	run_with_shutdown(transcriber, cancellation_token).await
}

struct Transcriber {
	config: Config,
	state: Arc<TranscriberState>,
	metrics: observability::TranscriberMetrics,
	transport: NatsTransport<UnifiedEvent>,
	queue_tx: mpsc::Sender<TranscriptionJob>,
	cancellation_token: CancellationToken,
}

async fn run_with_shutdown(transcriber: Transcriber, cancellation_token: CancellationToken) -> Result<()> {
	// Clone state for shutdown logging
	let state_for_shutdown = transcriber.state.clone();

	tokio::select! {
			result = transcriber.run() => {
					error!("Transcription loop exited unexpectedly: {:?}", result);
					result
			}
			_ = wait_for_shutdown_signal() => {
					info!("🛑 Shutdown signal received (SIGTERM/SIGINT)");

					// Signal all async tasks to stop
					cancellation_token.cancel();

					// Give async tasks a moment to notice cancellation and exit gracefully
					tokio::time::sleep(std::time::Duration::from_millis(SHUTDOWN_GRACE_PERIOD_MS)).await;

					// Log queue state at shutdown
					let jobs_enqueued = state_for_shutdown.jobs_enqueued.load(std::sync::atomic::Ordering::Relaxed);
					let jobs_dropped = state_for_shutdown.jobs_dropped.load(std::sync::atomic::Ordering::Relaxed);
					let queue_depth = state_for_shutdown.queue_depth.load(std::sync::atomic::Ordering::Relaxed);

					info!(
							jobs_enqueued,
							jobs_dropped,
							queue_depth,
							"📊 Shutdown statistics"
					);

					// DO NOT wait for blocking Whisper threads - they cannot be cancelled
					// The OS will clean them up when the process exits
					info!("✅ Exiting process (OS will clean up any remaining Whisper threads)");

					// Exit immediately - this is safe and correct for container environments
					std::process::exit(0);
			}
	}
}

async fn wait_for_shutdown_signal() {
	let ctrl_c = async {
		signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
	};

	#[cfg(unix)]
	let terminate = async {
		signal::unix::signal(signal::unix::SignalKind::terminate())
			.expect("failed to install SIGTERM handler")
			.recv()
			.await;
	};

	#[cfg(not(unix))]
	let terminate = std::future::pending::<()>();

	tokio::select! {
			_ = ctrl_c => {},
			_ = terminate => {},
	}
}

impl Transcriber {
	async fn run(self) -> Result<()> {
		let nats_client = self.transport.client();
		let subscriber = nats_client.subscribe(EventType::AudioChunk.subject()).await?;
		let nats_recv = NatsReceiver::<AudioChunkMessage>::new(subscriber);
		let mut receiver = TransportReceiver::new(nats_recv);

		info!("🎧 Subscribed to 'audio.chunk', waiting for audio...");

		let buffer_size = self.config.target_sample_rate as usize * self.config.buffer_duration_secs;
		let mut processor = audio::AudioProcessor::new(
			buffer_size,
			self.config.target_sample_rate,
			self.state.clone(),
			self.metrics.clone(),
			self.config.vad_enabled,
		);

		info!(
				target_sample_rate = self.config.target_sample_rate,
				buffer_duration_secs = self.config.buffer_duration_secs,
				buffer_size,
				whisper_threads = self.config.whisper_threads,
				queue_capacity = TRANSCRIPTION_QUEUE_CAPACITY,
				vad_enabled = self.config.vad_enabled,
				vad_threshold = self.config.vad_speech_threshold,
				vad_mode = ?self.config.get_vad_mode(),
				"📊 Configuration loaded"
		);

		loop {
			tokio::select! {
					_ = self.cancellation_token.cancelled() => {
							info!("🛑 Transcription loop cancelled");

							// Log final VAD stats
							if self.config.vad_enabled {
									if let Some(stats) = processor.vad_stats() {
											info!(
													vad_total_chunks = stats.total_chunks,
													vad_speech_chunks = stats.speech_chunks,
													vad_silence_chunks = stats.silence_chunks,
													vad_speech_ratio = format!("{:.1}%", stats.chunk_speech_ratio() * 100.0),
													"🎤 Final VAD statistics"
											);
									}
							}

							break;
					}
					result = tokio::time::timeout(
							std::time::Duration::from_millis(100),
							receiver.recv()
					) => {
							match result {
									Ok(Ok(audio_chunk)) => {
											if let Err(e) = self.process_audio_chunk(audio_chunk, &mut processor).await {
													error!(error = %e, "Failed to process audio chunk");
											}
									}
									Ok(Err(e)) => {
											error!(error = %e, "Failed to receive audio chunk");
											self.metrics.chunks_dropped.add(1, &[]);
									}
									Err(_) => {
											// Timeout - heartbeat check
											processor.heartbeat_check();
									}
							}
					}
			}
		}

		Ok(())
	}

	async fn process_audio_chunk(&self, audio_chunk: AudioChunkMessage, processor: &mut audio::AudioProcessor) -> Result<()> {
		// Decode samples from bytes
		let samples = audio_chunk.decode_samples().map_err(|e| anyhow::anyhow!("Failed to decode samples: {}", e))?;

		let sample_rate = audio_chunk.sample_rate.unwrap_or(48000);
		let channels = audio_chunk.channels.unwrap_or(2);

		processor.process_chunk(sample_rate, channels, samples).await?;

		// Check if ready to transcribe (VAD filtering happens inside take_buffer_if_ready)
		if let Some(audio_buffer) = processor.take_buffer_if_ready() {
			// Create transcription job
			let job = TranscriptionJob::new(
				0, // sequence number assigned by queue
				audio_buffer,
				self.config.target_sample_rate,
				None, // stream_id - could be extracted from audio_chunk if available
			);

			let audio_samples = job.audio.len();
			let audio_duration = job.audio_duration_secs();

			// Try to enqueue (non-blocking)
			match self.queue_tx.try_send(job) {
				Ok(_) => {
					self.state.increment_jobs_enqueued();
					self.metrics.transcription_jobs_enqueued.add(1, &[]);

					// Update queue depth (approximate)
					let current_depth = self.state.jobs_enqueued.load(std::sync::atomic::Ordering::Relaxed)
						- self.state.transcriptions_completed.load(std::sync::atomic::Ordering::Relaxed)
						- self.state.jobs_dropped.load(std::sync::atomic::Ordering::Relaxed);
					self.state.update_queue_depth(current_depth.min(TRANSCRIPTION_QUEUE_CAPACITY as u64) as usize);

					info!(audio_samples, queue_depth = current_depth, "✅ Job enqueued for transcription");
				}
				Err(mpsc::error::TrySendError::Full(_)) => {
					self.state.increment_jobs_dropped();
					self.metrics.transcription_jobs_dropped.add(1, &[]);

					warn!(
						audio_duration_secs = format!("{:.2}", audio_duration),
						queue_capacity = TRANSCRIPTION_QUEUE_CAPACITY,
						"⚠️ Transcription queue full - dropping audio buffer (backpressure active)"
					);
				}
				Err(mpsc::error::TrySendError::Closed(_)) => {
					warn!("⚠️ Transcription queue closed - system shutting down");
				}
			}
		}

		Ok(())
	}
}

async fn init_observability_with_fallback(config: &Config) -> (Option<opentelemetry_sdk::metrics::SdkMeterProvider>, observability::TranscriberMetrics) {
	// Vendors (OTLP/gRPC) already have retry logic built-in
	// We'll try once, and if it fails, continue with local-only metrics
	match observability::init_observability(&config.service_name) {
		Ok((provider, metrics)) => {
			info!("✅ Observability initialized with OTLP export");
			(Some(provider), metrics)
		}
		Err(e) => {
			warn!(
					error = %e,
					"⚠️ OTLP observability failed to initialize, falling back to local metrics only"
			);
			warn!("   Traces and metrics will NOT be exported (service will continue)");

			// Create local-only metrics (no export)
			let metrics = observability::create_local_metrics();
			(None, metrics)
		}
	}
}

async fn connect_with_retry(config: &Config) -> Result<NatsTransport<UnifiedEvent>> {
	for attempt in 1..=NATS_MAX_RETRIES {
		match NatsTransport::<UnifiedEvent>::connect_pooled(&config.nats_url).await {
			Ok(transport) => {
				info!(url = %config.nats_url, "✅ Connected to NATS");
				return Ok(transport);
			}
			Err(e) => {
				if attempt == NATS_MAX_RETRIES {
					error!(
							error = %e,
							url = %config.nats_url,
							"❌ Failed to connect to NATS after {} attempts - service cannot continue",
							NATS_MAX_RETRIES
					);
					return Err(e.into());
				}

				let backoff = NATS_INITIAL_BACKOFF_MS * 2_u64.pow(attempt - 1);
				warn!(
						attempt,
						max_retries = NATS_MAX_RETRIES,
						backoff_ms = backoff,
						error = %e,
						"⚠️ NATS connection failed, retrying..."
				);

				tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
			}
		}
	}

	unreachable!()
}

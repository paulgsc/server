mod audio;
mod config;
mod observability;
mod state;
mod transcription;

use anyhow::Result;
use clap::Parser;
use some_transport::{NatsReceiver, NatsTransport, TransportReceiver};
use std::sync::Arc;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_events::events::{AudioChunkMessage, EventType, UnifiedEvent};

use config::Config;
use state::TranscriberState;

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
			"🎯 Starting transcriber service"
	);

	// Initialize state
	let state = TranscriberState::new();
	state.register_gauges()?;

	// Connect to transport with retry
	let transport = connect_with_retry(&config).await?;

	// Load Whisper model
	let whisper_ctx = transcription::load_model(&config.whisper_model_path, config.whisper_threads)?;

	// Create cancellation token for cooperative shutdown
	let cancellation_token = CancellationToken::new();

	// Start transcription loop
	let transcriber = Transcriber {
		config,
		state,
		metrics,
		transport,
		whisper_ctx: Arc::new(whisper_ctx),
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
	whisper_ctx: Arc<whisper_rs::WhisperContext>,
	cancellation_token: CancellationToken,
}

async fn run_with_shutdown(transcriber: Transcriber, cancellation_token: CancellationToken) -> Result<()> {
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
		let transcription_sem = Arc::new(tokio::sync::Semaphore::new(1));

		let mut processor = audio::AudioProcessor::new(buffer_size, self.config.target_sample_rate, self.state.clone(), self.metrics.clone());

		let params = transcription::create_params(self.config.whisper_threads);

		info!(
			target_sample_rate = self.config.target_sample_rate,
			buffer_duration_secs = self.config.buffer_duration_secs,
			buffer_size,
			whisper_threads = self.config.whisper_threads,
			"📊 Configuration loaded"
		);

		loop {
			tokio::select! {
				_ = self.cancellation_token.cancelled() => {
					info!("🛑 Transcription loop cancelled");
					break;
				}
				result = tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()) => {
					match result {
						Ok(Ok(audio_chunk)) => {
							// audio_chunk is already AudioChunkMessage!
							if let Err(e) = self.process_audio_chunk(audio_chunk, &mut processor, &params, &transcription_sem).await {
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

	async fn process_audio_chunk(
		&self,
		audio_chunk: AudioChunkMessage,
		processor: &mut audio::AudioProcessor,
		params: &whisper_rs::FullParams<'static, 'static>,
		transcription_sem: &Arc<tokio::sync::Semaphore>,
	) -> Result<()> {
		// Decode samples from bytes
		let samples = audio_chunk.decode_samples().map_err(|e| anyhow::anyhow!("Failed to decode samples: {}", e))?;

		let sample_rate = audio_chunk.sample_rate.unwrap_or(48000);
		let channels = audio_chunk.channels.unwrap_or(2);

		processor.process_chunk(sample_rate, channels, samples).await?;

		// Check if ready to transcribe
		if let Some(audio_buffer) = processor.take_buffer_if_ready() {
			if let Ok(permit) = transcription_sem.clone().try_acquire_owned() {
				self.state.set_transcribing(true);

				info!(audio_samples = audio_buffer.len(), "🎤 Starting transcription");

				// Spawn and forget - don't track the handle
				// On shutdown, these tasks will be abandoned and cleaned up by the OS
				transcription::transcribe_and_publish(
					self.whisper_ctx.clone(),
					params.clone(),
					self.transport.clone(),
					audio_buffer,
					self.state.clone(),
					self.metrics.clone(),
					permit,
					self.cancellation_token.child_token(),
				);
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

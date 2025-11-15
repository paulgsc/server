use anyhow::Result;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{
	global,
	metrics::{Counter, Histogram, Meter, ObservableGauge},
	KeyValue,
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
	metrics::SdkMeterProvider,
	runtime,
	trace::{Config, RandomIdGenerator, Sampler},
	Resource,
};
use std::time::Duration;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Metrics for the transcriber service
#[derive(Clone)]
#[allow(dead_code)]
pub struct TranscriberMetrics {
	// Counters
	pub chunks_received: Counter<u64>,
	pub chunks_dropped: Counter<u64>,
	pub transcriptions_completed: Counter<u64>,
	pub transcriptions_failed: Counter<u64>,
	pub subtitles_published: Counter<u64>,
	pub bytes_received: Counter<u64>,
	pub samples_processed: Counter<u64>,

	// Histograms
	pub chunk_processing_latency: Histogram<f64>,
	pub transcription_latency: Histogram<f64>,
	pub buffer_fill_time: Histogram<f64>,
	pub resampling_latency: Histogram<f64>,

	// Gauges (will be set via callbacks)
	pub audio_buffer_size: ObservableGauge<u64>,
	pub current_sample_rate: ObservableGauge<u64>,
	pub heartbeat: ObservableGauge<u64>,
}

impl TranscriberMetrics {
	pub fn new(meter: &Meter) -> Self {
		Self {
			// Counters
			chunks_received: meter
				.u64_counter("transcriber.chunks.received")
				.with_description("Total audio chunks received from NATS")
				.init(),
			chunks_dropped: meter
				.u64_counter("transcriber.chunks.dropped")
				.with_description("Total chunks dropped due to errors")
				.init(),
			transcriptions_completed: meter
				.u64_counter("transcriber.transcriptions.completed")
				.with_description("Total successful transcriptions")
				.init(),
			transcriptions_failed: meter
				.u64_counter("transcriber.transcriptions.failed")
				.with_description("Total failed transcriptions")
				.init(),
			subtitles_published: meter
				.u64_counter("transcriber.subtitles.published")
				.with_description("Total subtitles published to NATS")
				.init(),
			bytes_received: meter
				.u64_counter("transcriber.bytes.received")
				.with_description("Total bytes of audio data received")
				.init(),
			samples_processed: meter.u64_counter("transcriber.samples.processed").with_description("Total audio samples processed").init(),

			// Histograms
			chunk_processing_latency: meter
				.f64_histogram("transcriber.chunk.latency")
				.with_description("Time to process each audio chunk (ms)")
				.init(),
			transcription_latency: meter
				.f64_histogram("transcriber.transcription.latency")
				.with_description("Time to transcribe audio buffer (ms)")
				.init(),
			buffer_fill_time: meter
				.f64_histogram("transcriber.buffer.fill_time")
				.with_description("Time to fill audio buffer to trigger transcription (seconds)")
				.init(),
			resampling_latency: meter.f64_histogram("transcriber.resampling.latency").with_description("Time to resample audio (ms)").init(),

			// Gauges
			audio_buffer_size: meter
				.u64_observable_gauge("transcriber.buffer.size")
				.with_description("Current audio buffer size in samples")
				.init(),
			current_sample_rate: meter.u64_observable_gauge("transcriber.sample_rate").with_description("Current audio sample rate").init(),
			heartbeat: meter
				.u64_observable_gauge("transcriber.heartbeat")
				.with_description("Last heartbeat timestamp (unix seconds)")
				.init(),
		}
	}
}

/// Initialize OpenTelemetry with OTLP exporter
pub fn init_observability(service_name: &str) -> Result<(SdkMeterProvider, TranscriberMetrics)> {
	// Get OTLP endpoint from env (default to localhost)
	let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4317".to_string());

	info!("🔧 Initializing OpenTelemetry");
	info!("   Service: {}", service_name);
	info!("   OTLP Endpoint: {}", otlp_endpoint);

	// Create resource with service metadata
	let resource = Resource::new(vec![
		KeyValue::new("service.name", service_name.to_string()),
		KeyValue::new("service.version", env!("CARGO_PKG_VERSION").to_string()),
	]);

	// Initialize tracing (spans)
	let tracer = opentelemetry_otlp::new_pipeline()
		.tracing()
		.with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(&otlp_endpoint))
		.with_trace_config(
			Config::default()
				.with_sampler(Sampler::AlwaysOn)
				.with_id_generator(RandomIdGenerator::default())
				.with_resource(resource.clone()),
		)
		.install_batch(runtime::Tokio)
		.map_err(|e| anyhow::anyhow!("Failed to initialize tracer: {}", e))?;

	// Get tracer instance (not provider)
	let tracer = tracer.tracer("transcriber");

	// Initialize metrics
	let meter_provider = opentelemetry_otlp::new_pipeline()
		.metrics(runtime::Tokio)
		.with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(&otlp_endpoint))
		.with_resource(resource)
		.with_period(Duration::from_secs(10)) // Export metrics every 10 seconds
		.build()
		.map_err(|e| anyhow::anyhow!("Failed to initialize metrics: {}", e))?;

	// Set global meter provider
	global::set_meter_provider(meter_provider.clone());

	// Create meter for this service
	let meter = global::meter(service_name.to_owned());

	// Create metrics
	let metrics = TranscriberMetrics::new(&meter);

	// Initialize tracing subscriber with OpenTelemetry layer
	let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

	let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,transcriber=debug,some_transport=debug"));

	tracing_subscriber::registry()
		.with(env_filter)
		.with(telemetry_layer)
		.with(tracing_subscriber::fmt::layer().with_target(true))
		.init();

	info!("✅ OpenTelemetry initialized successfully");

	Ok((meter_provider, metrics))
}

/// Create local-only metrics when OTLP export fails
/// This allows the service to continue operating without remote observability
pub fn create_local_metrics() -> TranscriberMetrics {
	use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

	// Initialize basic tracing without OpenTelemetry
	let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,transcriber=debug,some_transport=debug"));

	tracing_subscriber::registry()
		.with(env_filter)
		.with(tracing_subscriber::fmt::layer().with_target(true))
		.init();

	// Create a local meter (metrics will be tracked but not exported)
	let meter = global::meter("transcriber-local");
	TranscriberMetrics::new(&meter)
}

/// Heartbeat logger - call this periodically to track service health
pub struct Heartbeat {
	last_heartbeat: std::time::Instant,
	interval: Duration,
}

impl Heartbeat {
	pub fn new(interval_secs: u64) -> Self {
		Self {
			last_heartbeat: std::time::Instant::now(),
			interval: Duration::from_secs(interval_secs),
		}
	}

	/// Check if it's time for a heartbeat and log stats if so
	pub fn maybe_log(&mut self, chunks_received: u64, bytes_received: u64, samples_processed: u64, buffer_size: usize, transcriptions_completed: u64) -> bool {
		if self.last_heartbeat.elapsed() >= self.interval {
			info!(chunks_received, bytes_received, samples_processed, buffer_size, transcriptions_completed, "💓 Heartbeat");
			self.last_heartbeat = std::time::Instant::now();
			true
		} else {
			false
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_heartbeat_timing() {
		let mut heartbeat = Heartbeat::new(1);
		assert!(!heartbeat.maybe_log(0, 0, 0, 0, 0));
		std::thread::sleep(Duration::from_secs(1));
		assert!(heartbeat.maybe_log(0, 0, 0, 0, 0));
	}
}

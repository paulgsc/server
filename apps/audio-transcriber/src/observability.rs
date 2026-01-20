use anyhow::Result;
use opentelemetry::{
	global,
	metrics::{Counter, Histogram, Meter, ObservableGauge},
	trace::TracerProvider as _, // Required for .tracer()
	KeyValue,
};
use opentelemetry_otlp::WithExportConfig; // Required for .with_endpoint()
use opentelemetry_sdk::{
	metrics::{PeriodicReader, SdkMeterProvider},
	trace::{Sampler, SdkTracerProvider},
	Resource,
};
use std::{
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc,
	},
	time::Duration,
};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// ==========================
/// Atomic state for gauges
/// ==========================
#[derive(Clone, Default)]
pub struct GaugeState {
	pub audio_buffer_size: Arc<AtomicU64>,
	pub current_sample_rate: Arc<AtomicU64>,
	pub heartbeat_ts: Arc<AtomicU64>,
}

/// ==========================
/// Metrics for the transcriber
/// ==========================
#[derive(Clone)]
#[allow(dead_code)]
pub struct TranscriberMetrics {
	pub chunks_received: Counter<u64>,
	pub chunks_dropped: Counter<u64>,
	pub transcriptions_completed: Counter<u64>,
	pub transcriptions_failed: Counter<u64>,
	pub subtitles_published: Counter<u64>,
	pub bytes_received: Counter<u64>,
	pub samples_processed: Counter<u64>,
	pub transcription_jobs_enqueued: Counter<u64>,
	pub transcription_jobs_dropped: Counter<u64>,
	pub chunk_processing_latency: Histogram<f64>,
	pub transcription_latency: Histogram<f64>,
	pub buffer_fill_time: Histogram<f64>,
	pub resampling_latency: Histogram<f64>,
	pub transcription_queue_latency: Histogram<f64>,
	pub transcription_processing_latency: Histogram<f64>,
	pub transcription_end_to_end_latency: Histogram<f64>,
	pub audio_buffer_size: ObservableGauge<u64>,
	pub current_sample_rate: ObservableGauge<u64>,
	pub heartbeat: ObservableGauge<u64>,
	pub gauges: GaugeState,
}

impl TranscriberMetrics {
	pub fn new(meter: &Meter) -> Self {
		let gauges = GaugeState::default();

		let audio_buffer_state = gauges.audio_buffer_size.clone();
		let sample_rate_state = gauges.current_sample_rate.clone();
		let heartbeat_state = gauges.heartbeat_ts.clone();

		Self {
			chunks_received: meter.u64_counter("transcriber.chunks.received").with_description("Total audio chunks received").build(),
			chunks_dropped: meter.u64_counter("transcriber.chunks.dropped").build(),
			transcriptions_completed: meter.u64_counter("transcriber.transcriptions.completed").build(),
			transcriptions_failed: meter.u64_counter("transcriber.transcriptions.failed").build(),
			subtitles_published: meter.u64_counter("transcriber.subtitles.published").build(),
			bytes_received: meter.u64_counter("transcriber.bytes.received").build(),
			samples_processed: meter.u64_counter("transcriber.samples.processed").build(),
			transcription_jobs_enqueued: meter.u64_counter("transcriber.jobs.enqueued").build(),
			transcription_jobs_dropped: meter.u64_counter("transcriber.jobs.dropped").build(),
			chunk_processing_latency: meter.f64_histogram("transcriber.chunk.latency_ms").build(),
			transcription_latency: meter.f64_histogram("transcriber.transcription.latency_ms").build(),
			buffer_fill_time: meter.f64_histogram("transcriber.buffer.fill_time_s").build(),
			resampling_latency: meter.f64_histogram("transcriber.resampling.latency_ms").build(),
			transcription_queue_latency: meter.f64_histogram("transcriber.queue.latency_ms").build(),
			transcription_processing_latency: meter.f64_histogram("transcriber.processing.latency_ms").build(),
			transcription_end_to_end_latency: meter.f64_histogram("transcriber.end_to_end.latency_ms").build(),

			audio_buffer_size: meter
				.u64_observable_gauge("transcriber.buffer.size")
				.with_description("Audio buffer size (samples)")
				.with_callback(move |observer| {
					observer.observe(audio_buffer_state.load(Ordering::Relaxed), &[]);
				})
				.build(),

			current_sample_rate: meter
				.u64_observable_gauge("transcriber.sample_rate")
				.with_callback(move |observer| {
					observer.observe(sample_rate_state.load(Ordering::Relaxed), &[]);
				})
				.build(),

			heartbeat: meter
				.u64_observable_gauge("transcriber.heartbeat")
				.with_description("Last heartbeat unix timestamp")
				.with_callback(move |observer| {
					observer.observe(heartbeat_state.load(Ordering::Relaxed), &[]);
				})
				.build(),

			gauges,
		}
	}
}

/// ===================================
/// OpenTelemetry initialization (0.29)
/// ===================================
pub fn init_observability(service_name: &str) -> Result<(SdkMeterProvider, TranscriberMetrics)> {
	let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4317".into());
	// Leak once so we can reuse the static reference for both tracer and meter
	let static_name: &'static str = Box::leak(service_name.to_string().into_boxed_str());

	info!("🔧 Initializing OpenTelemetry 0.29");
	info!("   Service: {service_name} | Endpoint: {otlp_endpoint}");

	let resource = Resource::builder()
		.with_attributes(vec![
			KeyValue::new("service.name", service_name.to_string()),
			KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
		])
		.build();

	// ---------- Tracing ----------
	let span_exporter = opentelemetry_otlp::SpanExporter::builder().with_tonic().with_endpoint(&otlp_endpoint).build()?;

	let tracer_provider = SdkTracerProvider::builder()
		.with_sampler(Sampler::AlwaysOn)
		.with_resource(resource.clone())
		.with_batch_exporter(span_exporter)
		.build();

	let tracer = tracer_provider.tracer("transcriber");
	global::set_tracer_provider(tracer_provider);

	// ---------- Metrics ----------
	let metric_exporter = opentelemetry_otlp::MetricExporter::builder().with_tonic().with_endpoint(&otlp_endpoint).build()?;

	let reader = PeriodicReader::builder(metric_exporter).with_interval(Duration::from_secs(10)).build();

	let meter_provider = SdkMeterProvider::builder().with_resource(resource).with_reader(reader).build();

	global::set_meter_provider(meter_provider.clone());

	let meter = global::meter(static_name);
	let metrics = TranscriberMetrics::new(&meter);

	// ---------- Tracing subscriber ----------
	let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,audio_transcriber=debug"));

	tracing_subscriber::registry()
		.with(env_filter)
		.with(tracing_opentelemetry::layer().with_tracer(tracer))
		.with(tracing_subscriber::fmt::layer().with_target(true))
		.init();

	info!("✅ OpenTelemetry initialized successfully");

	Ok((meter_provider, metrics))
}

pub fn create_local_metrics() -> TranscriberMetrics {
	let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,audio_transcriber=debug"));

	let _ = tracing_subscriber::registry()
		.with(env_filter)
		.with(tracing_subscriber::fmt::layer().with_target(true))
		.try_init();

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

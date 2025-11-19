//! OpenTelemetry initialization and configuration
//!
//! This module sets up distributed tracing and metrics for the file_host service.
//! All business logic modules automatically participate once this is initialized.

use opentelemetry::{global, trace::TracerProvider as _, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
	metrics::{self, reader::DefaultAggregationSelector, reader::DefaultTemporalitySelector},
	runtime,
	trace::{self, RandomIdGenerator, Sampler},
	Resource,
};
use opentelemetry_semantic_conventions as semconv;
use std::time::Duration;
use thiserror::Error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Error, Debug)]
pub enum ObservabilityError {
	#[error("Failed to initialize OTLP tracer: {0}")]
	TracerInit(#[from] opentelemetry::trace::TraceError),

	#[error("Failed to initialize OTLP metrics: {0}")]
	MetricsInit(#[from] opentelemetry::metrics::MetricsError),
}

/// Cached configuration read once at startup
struct OtelConfig {
	service_name: String,
	otlp_endpoint: String,
	sampler: Sampler,
	environment: String,
	metrics_export_interval_secs: u64,
	metrics_export_timeout_secs: u64,
}

impl OtelConfig {
	fn from_env() -> Self {
		Self {
			service_name: std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "file_host".to_string()),
			otlp_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4317".to_string()),
			sampler: Self::sampler_from_env(),
			environment: std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()),
			metrics_export_interval_secs: std::env::var("OTEL_METRIC_EXPORT_INTERVAL").ok().and_then(|s| s.parse().ok()).unwrap_or(60),
			metrics_export_timeout_secs: std::env::var("OTEL_METRIC_EXPORT_TIMEOUT").ok().and_then(|s| s.parse().ok()).unwrap_or(30),
		}
	}

	fn sampler_from_env() -> Sampler {
		match std::env::var("OTEL_TRACES_SAMPLER").as_deref() {
			Ok("always_on") => Sampler::AlwaysOn,
			Ok("always_off") => Sampler::AlwaysOff,
			Ok("traceidratio") | _ => {
				let ratio = std::env::var("OTEL_TRACES_SAMPLER_ARG").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
				Sampler::TraceIdRatioBased(ratio)
			}
		}
	}
}

/// Initialize OpenTelemetry tracing and metrics
///
/// This should be called once at service startup, before any business logic runs.
///
/// # Environment Variables
/// - `OTEL_EXPORTER_OTLP_ENDPOINT`: OTLP endpoint (default: http://localhost:4317)
/// - `OTEL_SERVICE_NAME`: Service name (default: file_host)
/// - `OTEL_TRACES_SAMPLER`: Sampler type: always_on, always_off, traceidratio (default: traceidratio)
/// - `OTEL_TRACES_SAMPLER_ARG`: Sampling ratio 0.0-1.0 (default: 1.0 = 100%)
/// - `OTEL_METRIC_EXPORT_INTERVAL`: Metrics export interval in seconds (default: 60)
/// - `OTEL_METRIC_EXPORT_TIMEOUT`: Metrics export timeout in seconds (default: 30)
/// - `RUST_LOG`: Log level filter (default: info)
pub fn init() -> Result<OtelGuard, ObservabilityError> {
	let config = OtelConfig::from_env();

	// 1. Create resource with service information
	let resource = Resource::new(vec![
		KeyValue::new(semconv::resource::SERVICE_NAME, config.service_name.clone()),
		KeyValue::new(semconv::resource::SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
		KeyValue::new("service.environment", config.environment.clone()),
	]);

	// 2. Setup tracing pipeline (exports to Tempo/Jaeger via OTLP)
	let tracer = opentelemetry_otlp::new_pipeline()
		.tracing()
		.with_exporter(
			opentelemetry_otlp::new_exporter()
				.tonic()
				.with_endpoint(&config.otlp_endpoint)
				.with_timeout(Duration::from_secs(3)),
		)
		.with_trace_config(
			trace::config()
				.with_sampler(config.sampler)
				.with_id_generator(RandomIdGenerator::default())
				.with_resource(resource.clone()),
		)
		.install_batch(runtime::Tokio)?;

	// 3. Setup metrics pipeline (exports to Prometheus via OTLP)
	let metrics_reader = opentelemetry_otlp::new_pipeline()
		.metrics(runtime::Tokio)
		.with_exporter(
			opentelemetry_otlp::new_exporter()
				.tonic()
				.with_endpoint(&config.otlp_endpoint)
				.with_timeout(Duration::from_secs(config.metrics_export_timeout_secs)),
		)
		.with_resource(resource)
		.with_period(Duration::from_secs(config.metrics_export_interval_secs))
		.build()?;

	global::set_meter_provider(metrics_reader.clone());

	// 4. Setup tracing subscriber with multiple layers
	let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer.tracer(&config.service_name));

	let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

	tracing_subscriber::registry()
		.with(env_filter)
		.with(telemetry_layer)
		.with(tracing_subscriber::fmt::layer().with_target(true))
		.init();

	tracing::info!(
			service_name = %config.service_name,
			otlp_endpoint = %config.otlp_endpoint,
			sampler = ?config.sampler,
			metrics_export_interval_secs = %config.metrics_export_interval_secs,
			"OpenTelemetry initialized"
	);

	Ok(OtelGuard { meter_provider: metrics_reader })
}

/// Guard that handles graceful shutdown of OTel resources
pub struct OtelGuard {
	meter_provider: opentelemetry_sdk::metrics::SdkMeterProvider,
}

impl OtelGuard {
	/// Shutdown metrics provider gracefully
	pub async fn shutdown(&self) -> Result<(), ObservabilityError> {
		self.meter_provider.shutdown()?;
		Ok(())
	}
}

impl Drop for OtelGuard {
	fn drop(&mut self) {
		tracing::info!("Shutting down OpenTelemetry");
		global::shutdown_tracer_provider();
		// Note: meter provider shutdown should be called explicitly via shutdown()
		// before drop for proper async cleanup
	}
}

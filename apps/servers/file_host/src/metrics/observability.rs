use opentelemetry::{global, trace::TracerProvider as _, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
	metrics::{PeriodicReader, SdkMeterProvider},
	trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
	Resource,
};
use std::time::Duration;
use thiserror::Error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Error, Debug)]
pub enum ObservabilityError {
	#[error("Failed to initialize OTLP tracer: {0}")]
	TracerInit(#[from] opentelemetry_sdk::trace::TraceError),

	#[error("Failed to initialize OTLP exporter: {0}")]
	ExporterInit(#[from] opentelemetry_otlp::ExporterBuildError),

	#[error("Failed to initialize OTLP metrics: {0}")]
	MetricsInit(String),

	#[error("OpenTelemetry error: {0}")]
	OpenTelemetry(String),
}

pub struct OtelGuard {
	tracer_provider: SdkTracerProvider,
	meter_provider: SdkMeterProvider,
}

impl OtelGuard {
	/// Create and initialize OpenTelemetry with tracing subscriber
	pub fn new() -> Result<Self, ObservabilityError> {
		let config = OtelConfig::from_env();

		let resource = Resource::builder()
			.with_service_name(config.service_name.clone())
			.with_attributes(vec![
				KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
				KeyValue::new("service.environment", config.environment.clone()),
			])
			.build();

		// === Tracing ===
		let trace_exporter = opentelemetry_otlp::SpanExporter::builder()
			.with_tonic()
			.with_endpoint(&config.otlp_endpoint)
			.with_timeout(Duration::from_secs(3))
			.build()?;

		let tracer_provider = SdkTracerProvider::builder()
			.with_resource(resource.clone())
			.with_sampler(config.sampler.clone())
			.with_id_generator(RandomIdGenerator::default())
			.with_batch_exporter(trace_exporter)
			.build();

		let tracer = tracer_provider.tracer(config.service_name.clone());

		global::set_tracer_provider(tracer_provider.clone());

		// === Metrics ===
		let metrics_exporter = opentelemetry_otlp::MetricExporter::builder()
			.with_tonic()
			.with_endpoint(&config.otlp_endpoint)
			.with_timeout(Duration::from_secs(config.metrics_export_timeout_secs))
			.build()?;

		let reader = PeriodicReader::builder(metrics_exporter)
			.with_interval(Duration::from_secs(config.metrics_export_interval_secs))
			.build();

		let meter_provider = SdkMeterProvider::builder().with_resource(resource).with_reader(reader).build();

		global::set_meter_provider(meter_provider.clone());

		// === Tracing subscriber ===
		let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);
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

		Ok(Self { tracer_provider, meter_provider })
	}

	/// Shutdown OpenTelemetry providers
	/// Note: This consumes self because shutdown needs to take ownership
	pub async fn shutdown(self) -> Result<(), ObservabilityError> {
		self.tracer_provider.shutdown().map_err(|e| ObservabilityError::OpenTelemetry(e.to_string()))?;
		self.meter_provider.shutdown().map_err(|e| ObservabilityError::OpenTelemetry(e.to_string()))?;
		Ok(())
	}
}

impl Drop for OtelGuard {
	fn drop(&mut self) {
		tracing::info!("OtelGuard dropped (use shutdown() for proper async cleanup)");
	}
}

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
			_ => {
				let ratio = std::env::var("OTEL_TRACES_SAMPLER_ARG").ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(1.0);
				Sampler::TraceIdRatioBased(ratio)
			}
		}
	}
}

use opentelemetry::{global, trace::TracerProvider as _, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
	trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
	Resource,
};
use std::time::Duration;
use thiserror::Error;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
	layer::{Context, Layer, SubscriberExt},
	registry::LookupSpan,
	util::SubscriberInitExt,
	EnvFilter,
};

/// Mirrors every `ERROR`-level `tracing` event, workspace-wide, into
/// `tracing_events_total{target}` — the same generalization this repo
/// already applies to `operation_errors_total` (metrics/instruments.rs), but
/// keyed on tracing call-site metadata instead of a hand-picked operation
/// name, so a fault in a module nobody's gotten around to instrumenting yet
/// (a WS handler, a REST route, a background task) still produces a metric
/// an alert can fire on.
///
/// `target` is `metadata.target()` — a `&'static str` fixed by the call
/// site's module path, not event content — so the label set is bounded by
/// the code that ships, never by what an event happens to log. This is the
/// same cardinality discipline `operation_errors_total`'s callers already
/// follow: labels are a small closed set of categories, never raw external
/// strings. Only `ERROR` is mirrored; `WARN` (routine 4xx responses,
/// `error.rs`'s `into_response` among them — see its own comment) is a
/// normal outcome, not a fault, and mirroring it here would make this
/// metric fire on ordinary client traffic instead of on the operational
/// faults it exists to page on.
struct ErrorEventMetricsLayer;

impl<S> Layer<S> for ErrorEventMetricsLayer
where
	S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
	fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
		let metadata = event.metadata();
		if metadata.level() == &Level::ERROR {
			metrics::counter!("tracing_events_total", "level" => "error", "target" => metadata.target()).increment(1);
		}
	}
}

#[derive(Error, Debug)]
pub enum ObservabilityError {
	#[error("Failed to initialize OTLP tracer: {0}")]
	TracerInit(#[from] opentelemetry_sdk::trace::TraceError),

	#[error("Failed to initialize OTLP exporter: {0}")]
	ExporterInit(#[from] opentelemetry_otlp::ExporterBuildError),

	#[error("OpenTelemetry error: {0}")]
	OpenTelemetry(String),
}

/// Tracing-only. Per #139 (P1), application measurements go through the
/// `metrics` facade and a Prometheus scrape (see `metrics::instruments`
/// and the `some-metrics` seam crate), not OTLP — this guard no longer
/// carries a meter provider.
pub struct OtelGuard {
	tracer_provider: SdkTracerProvider,
}

impl OtelGuard {
	/// Create and initialize `OpenTelemetry` tracing with the tracing subscriber.
	pub fn new() -> Result<Self, ObservabilityError> {
		let config = OtelConfig::from_env();

		let resource = Resource::builder()
			.with_service_name(config.service_name.clone())
			.with_attributes(vec![
				KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
				KeyValue::new("service.environment", config.environment.clone()),
			])
			.build();

		let trace_exporter = opentelemetry_otlp::SpanExporter::builder()
			.with_tonic()
			.with_endpoint(&config.otlp_endpoint)
			.with_timeout(Duration::from_secs(3))
			.build()?;

		let tracer_provider = SdkTracerProvider::builder()
			.with_resource(resource)
			.with_sampler(config.sampler.clone())
			.with_id_generator(RandomIdGenerator::default())
			.with_batch_exporter(trace_exporter)
			.build();

		let tracer = tracer_provider.tracer(config.service_name.clone());

		global::set_tracer_provider(tracer_provider.clone());

		let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);
		let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

		tracing_subscriber::registry()
			.with(env_filter)
			.with(ErrorEventMetricsLayer)
			.with(telemetry_layer)
			.with(tracing_subscriber::fmt::layer().with_target(true))
			.init();

		tracing::info!(
			service_name = %config.service_name,
			otlp_endpoint = %config.otlp_endpoint,
			sampler = ?config.sampler,
			"OpenTelemetry tracing initialized"
		);

		Ok(Self { tracer_provider })
	}

	/// Shutdown the tracer provider.
	/// Note: This consumes self because shutdown needs to take ownership
	pub async fn shutdown(self) -> Result<(), ObservabilityError> {
		self.tracer_provider.shutdown().map_err(|e| ObservabilityError::OpenTelemetry(e.to_string()))?;
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
}

impl OtelConfig {
	fn from_env() -> Self {
		Self {
			service_name: std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "file_host".to_string()),
			otlp_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4317".to_string()),
			sampler: Self::sampler_from_env(),
			environment: std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()),
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

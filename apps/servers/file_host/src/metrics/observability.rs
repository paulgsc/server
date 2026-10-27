use crate::config::{OtelConfig, TraceSampler};
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
	#[error("Failed to initialize OTLP exporter: {0}")]
	ExporterInit(#[from] opentelemetry_otlp::ExporterBuildError),

	#[error("OpenTelemetry error: {0}")]
	OpenTelemetry(String),

	#[error("RUST_LOG: {0}")]
	Filter(#[from] tracing_subscriber::filter::ParseError),
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
	///
	/// Both arguments are `Config`'s: this module reads no environment (#484).
	///
	/// # Errors
	///
	/// Fails when the OTLP exporter cannot be built, or `rust_log` does not
	/// parse (`Config` has already refused one that does not).
	pub fn new(config: &OtelConfig, rust_log: &str) -> Result<Self, ObservabilityError> {
		let sampler = sampler(config);

		let resource = Resource::builder()
			.with_service_name(config.otel_service_name.clone())
			.with_attributes(vec![
				KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
				KeyValue::new("service.environment", config.environment.clone()),
			])
			.build();

		let trace_exporter = opentelemetry_otlp::SpanExporter::builder()
			.with_tonic()
			.with_endpoint(&config.otel_exporter_otlp_endpoint)
			.with_timeout(Duration::from_secs(3))
			.build()?;

		let tracer_provider = SdkTracerProvider::builder()
			.with_resource(resource)
			.with_sampler(sampler.clone())
			.with_id_generator(RandomIdGenerator::default())
			.with_batch_exporter(trace_exporter)
			.build();

		let tracer = tracer_provider.tracer(config.otel_service_name.clone());

		global::set_tracer_provider(tracer_provider.clone());

		let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);
		let env_filter = EnvFilter::try_new(rust_log)?;

		tracing_subscriber::registry()
			.with(env_filter)
			// Whatever RUST_LOG says: the WebAuthn library's own debug spans
			// record whole credentials. See `auth::loggable`.
			.with(tracing_subscriber::filter::filter_fn(crate::auth::loggable))
			.with(ErrorEventMetricsLayer)
			.with(telemetry_layer)
			.with(tracing_subscriber::fmt::layer().with_target(true))
			.init();

		tracing::info!(
			service_name = %config.otel_service_name,
			otlp_endpoint = %config.otel_exporter_otlp_endpoint,
			sampler = ?sampler,
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

/// The sampler `OTEL_TRACES_SAMPLER` and `OTEL_TRACES_SAMPLER_ARG` name.
const fn sampler(config: &OtelConfig) -> Sampler {
	match config.otel_traces_sampler {
		TraceSampler::AlwaysOn => Sampler::AlwaysOn,
		TraceSampler::AlwaysOff => Sampler::AlwaysOff,
		TraceSampler::Traceidratio => Sampler::TraceIdRatioBased(config.otel_traces_sampler_arg),
	}
}

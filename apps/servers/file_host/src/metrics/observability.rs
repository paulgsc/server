use app_config::{OtelArgs, TraceSampler};
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
	/// `None` when no collector is configured: there is then no OpenTelemetry
	/// pipeline at all, so nothing can fail to export (#545).
	tracer_provider: Option<SdkTracerProvider>,
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
	pub fn new(config: &OtelArgs, rust_log: &str) -> Result<Self, ObservabilityError> {
		let (guard, subscriber) = Self::build(config, rust_log)?;
		if let Some(tracer_provider) = &guard.tracer_provider {
			global::set_tracer_provider(tracer_provider.clone());
		}
		subscriber.init();

		tracing::info!(
			service_name = %config.otel_service_name,
			otlp_endpoint = collector_endpoint(config).unwrap_or("none: traces are not exported"),
			sampler = ?sampler(config),
			"OpenTelemetry tracing initialized"
		);

		Ok(guard)
	}

	/// The guard and the subscriber [`OtelGuard::new`] installs, without
	/// installing either, so a test can run the same subscriber.
	///
	/// With no collector configured (`OTEL_EXPORTER_OTLP_ENDPOINT` unset or
	/// empty) the subscriber has no OpenTelemetry layer: spans are still made
	/// by `tracing`, for the log lines that carry them, and exported nowhere.
	/// An exporter there would fail every batch, at ERROR, every 5 s of use,
	/// since no collector runs (#545). With one configured, a failed export is
	/// still the SDK's ERROR event, counted and alerted on like any other.
	fn build(config: &OtelArgs, rust_log: &str) -> Result<(Self, impl Subscriber + Send + Sync + 'static), ObservabilityError> {
		let tracer_provider = exporting_tracer_provider(config)?;
		let telemetry_layer = tracer_provider
			.as_ref()
			.map(|tracer_provider| tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer(config.otel_service_name.clone())));
		let env_filter = EnvFilter::try_new(rust_log)?;

		let subscriber = tracing_subscriber::registry()
			.with(env_filter)
			// Whatever RUST_LOG says: the WebAuthn library's own debug spans
			// record whole credentials. See `auth::loggable`.
			.with(tracing_subscriber::filter::filter_fn(crate::auth::loggable))
			.with(ErrorEventMetricsLayer)
			.with(telemetry_layer)
			.with(tracing_subscriber::fmt::layer().with_target(true));

		Ok((Self { tracer_provider }, subscriber))
	}

	/// Flushes what is left to the collector and shuts the exporter down, if
	/// there is one. Consumes `self`: nothing is exported after it.
	///
	/// # Errors
	///
	/// When the last export or the shutdown itself fails.
	pub fn shutdown(self) -> Result<(), ObservabilityError> {
		if let Some(tracer_provider) = &self.tracer_provider {
			tracer_provider.shutdown().map_err(|e| ObservabilityError::OpenTelemetry(e.to_string()))?;
		}
		Ok(())
	}
}

impl Drop for OtelGuard {
	fn drop(&mut self) {
		tracing::info!("OtelGuard dropped (use shutdown() for proper async cleanup)");
	}
}

/// The collector `OTEL_EXPORTER_OTLP_ENDPOINT` names, if it names one.
///
/// Empty counts as unset, as it does everywhere else the config is read
/// (`app_config::schema`'s `SET`, the NixOS module's `isSet`): the generated
/// `example.env` line uncommented as it stands is `OTEL_EXPORTER_OTLP_ENDPOINT=""`.
/// Handed to the exporter, an empty endpoint is not "export nowhere": the
/// OTLP crate reads the variable itself, so an empty one stopped `file_host`
/// at start (`InvalidUri`), and an empty `--otel-exporter-otlp-endpoint`
/// with the variable unset exported to `http://localhost:4317`, where nothing
/// listens: the ERROR every 5 s of #545.
fn collector_endpoint(config: &OtelArgs) -> Option<&str> {
	config.otel_exporter_otlp_endpoint.as_deref().map(str::trim).filter(|endpoint| !endpoint.is_empty())
}

/// A tracer provider exporting to the configured collector, or `None` when
/// there is none to export to.
fn exporting_tracer_provider(config: &OtelArgs) -> Result<Option<SdkTracerProvider>, ObservabilityError> {
	let Some(endpoint) = collector_endpoint(config) else {
		return Ok(None);
	};

	let resource = Resource::builder()
		.with_service_name(config.otel_service_name.clone())
		.with_attributes(vec![
			KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
			KeyValue::new("service.environment", config.environment.clone()),
		])
		.build();
	let trace_exporter = opentelemetry_otlp::SpanExporter::builder()
		.with_tonic()
		.with_endpoint(endpoint)
		.with_timeout(Duration::from_secs(3))
		.build()?;

	Ok(Some(
		SdkTracerProvider::builder()
			.with_resource(resource)
			.with_sampler(sampler(config))
			.with_id_generator(RandomIdGenerator::default())
			.with_batch_exporter(trace_exporter)
			.build(),
	))
}

/// The sampler `OTEL_TRACES_SAMPLER` and `OTEL_TRACES_SAMPLER_ARG` name.
const fn sampler(config: &OtelArgs) -> Sampler {
	match config.otel_traces_sampler {
		TraceSampler::AlwaysOn => Sampler::AlwaysOn,
		TraceSampler::AlwaysOff => Sampler::AlwaysOff,
		TraceSampler::Traceidratio => Sampler::TraceIdRatioBased(config.otel_traces_sampler_arg),
	}
}

#[cfg(test)]
mod tests {
	use super::OtelGuard;
	use app_config::{OtelArgs, TraceSampler};
	use std::sync::{Mutex, Once};
	use tracing::{Event, Level, Subscriber};
	use tracing_subscriber::{
		filter::filter_fn,
		layer::{Context, Layer, SubscriberExt},
		Registry,
	};

	/// Every ERROR the OpenTelemetry crates log, as `(target, name)`, from any
	/// thread: the batch processor exports, and fails, on a thread of its
	/// own, which only the global subscriber sees.
	static OTEL_ERRORS: Mutex<Vec<(&str, &str)>> = Mutex::new(Vec::new());
	/// One test at a time reads `OTEL_ERRORS`.
	static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
	static GLOBAL: Once = Once::new();

	struct CaptureOtelErrors;

	impl<S: Subscriber> Layer<S> for CaptureOtelErrors {
		fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
			let metadata = event.metadata();
			if metadata.level() == &Level::ERROR && metadata.target().starts_with("opentelemetry") {
				OTEL_ERRORS.lock().unwrap().push((metadata.target(), metadata.name()));
			}
		}
	}

	/// The global subscriber sees only the OpenTelemetry crates' events, so
	/// it changes nothing for the other tests in this binary.
	fn capture_otel_errors_globally() {
		GLOBAL.call_once(|| {
			let subscriber = Registry::default().with(CaptureOtelErrors.with_filter(filter_fn(|metadata| metadata.target().starts_with("opentelemetry"))));
			tracing::subscriber::set_global_default(subscriber).unwrap();
		});
		OTEL_ERRORS.lock().unwrap().clear();
	}

	fn otel_args(endpoint: Option<&str>) -> OtelArgs {
		OtelArgs {
			otel_service_name: "file_host".into(),
			otel_exporter_otlp_endpoint: endpoint.map(str::to_owned),
			otel_traces_sampler: TraceSampler::AlwaysOn,
			otel_traces_sampler_arg: 1.0,
			environment: "test".into(),
		}
	}

	/// What a handler does: an `info` span, as `#[instrument]` makes, with a
	/// log line in it. Each one is a sampled span to export.
	fn handle_requests() {
		for _ in 0..3 {
			let span = tracing::info_span!("handler", otel.kind = "server");
			let _entered = span.enter();
			tracing::info!("handled");
		}
	}

	/// #545: with no collector configured, a run that makes spans exports
	/// none and logs no OpenTelemetry ERROR, through shutdown. Empty counts as
	/// unset: handed to the exporter, it would export to localhost:4317.
	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn with_no_collector_configured_a_run_logs_no_export_error() {
		let _serial = SERIAL.lock().await;
		for endpoint in [None, Some(""), Some("  ")] {
			capture_otel_errors_globally();
			let (guard, subscriber) = OtelGuard::build(&otel_args(endpoint), "info").unwrap();
			assert!(guard.tracer_provider.is_none(), "{endpoint:?} configured an exporter");

			tracing::subscriber::with_default(subscriber.with(CaptureOtelErrors), handle_requests);
			tokio::task::spawn_blocking(move || guard.shutdown()).await.unwrap().unwrap();

			let errors = OTEL_ERRORS.lock().unwrap().clone();
			assert!(errors.is_empty(), "{endpoint:?}: {errors:?}");
		}
	}

	/// A collector that is configured and down is not swallowed: the failed
	/// export is the SDK's ERROR event, which `ErrorEventMetricsLayer` counts
	/// and the error alert fires on.
	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn a_configured_collector_that_is_down_logs_the_export_error() {
		let _serial = SERIAL.lock().await;
		capture_otel_errors_globally();
		// A port nothing listens on: bound, then released.
		let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
		let endpoint = "http://127.0.0.1:".to_owned() + &port.to_string();
		let (guard, subscriber) = OtelGuard::build(&otel_args(Some(&endpoint)), "info").unwrap();
		let tracer_provider = guard.tracer_provider.clone().unwrap();

		tracing::subscriber::with_default(subscriber, handle_requests);
		let flushed = tokio::task::spawn_blocking(move || tracer_provider.force_flush()).await.unwrap();
		assert!(flushed.is_err(), "nothing listens on {port}");
		tokio::task::spawn_blocking(move || guard.shutdown()).await.unwrap().ok();

		let errors = OTEL_ERRORS.lock().unwrap().clone();
		assert!(errors.contains(&("opentelemetry_sdk", "BatchSpanProcessor.ExportError")), "{errors:?}");
	}
}

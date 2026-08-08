//! The one exposure path for application metrics — see [#139][issue-139].
//!
//! Application crates record measurements through the `metrics` facade
//! (`counter!`, `gauge!`, `histogram!`) and never import a Prometheus type
//! directly. This crate is infrastructure: it installs the one global
//! recorder those macros write to, and turns its accumulated state into a
//! Prometheus exposition payload on request.
//!
//! [issue-139]: https://github.com/paulgsc/server/issues/139

use axum::{extract::Extension, routing::get, Router};
use metrics_exporter_prometheus::{BuildError, PrometheusHandle};
use std::net::SocketAddr;

// Re-exported so a caller can configure the builder (histogram buckets,
// quantiles, …) before installing it, without taking its own direct
// dependency on `metrics-exporter-prometheus` — this crate already owns
// that version pin. See [`install_with`].
pub use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};

#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
	#[error("failed to install the metrics recorder: {0}")]
	Install(#[from] BuildError),
	#[error("failed to bind the metrics listener on {addr}: {source}")]
	Bind { addr: SocketAddr, source: std::io::Error },
	#[error("metrics listener on {addr} failed: {source}")]
	Serve { addr: SocketAddr, source: std::io::Error },
}

/// A handle to the installed recorder, cheap to clone and pass into a router
/// as state.
#[derive(Clone)]
pub struct MetricsHandle(PrometheusHandle);

impl MetricsHandle {
	/// Render the current state of every counter, gauge and histogram as a
	/// Prometheus text exposition payload.
	#[must_use]
	pub fn render(&self) -> String {
		self.0.render()
	}
}

/// Install the global `metrics` recorder, backed by a Prometheus exporter,
/// with the exporter's default configuration.
///
/// Call this exactly once per process, before anything records a
/// measurement. The returned handle is what turns the recorder's state into
/// a scrapeable payload — pass it to [`route`] or [`serve`].
///
/// # Errors
///
/// Returns [`MetricsError::Install`] if a global recorder is already
/// installed — this must be called at most once per process.
pub fn install() -> Result<MetricsHandle, MetricsError> {
	install_with(PrometheusBuilder::new())
}

/// Install the global `metrics` recorder from a caller-configured builder —
/// for a service that needs its own histogram buckets (default buckets top
/// out at 10s; a request-duration histogram sitting behind a 15s timeout
/// needs a bucket above it, or timeouts fall off the end of the histogram)
/// or quantiles. `PrometheusBuilder` and [`Matcher`] are re-exported from
/// this crate so a caller never needs its own direct
/// `metrics-exporter-prometheus` dependency.
///
/// Otherwise identical to [`install`] — same one-call-per-process rule, same
/// use of the returned handle.
///
/// # Errors
///
/// Returns [`MetricsError::Install`] if a global recorder is already
/// installed, or if `builder` was misconfigured (e.g. empty buckets).
pub fn install_with(builder: PrometheusBuilder) -> Result<MetricsHandle, MetricsError> {
	let handle = builder.install_recorder()?;
	Ok(MetricsHandle(handle))
}

async fn render(Extension(handle): Extension<MetricsHandle>) -> String {
	handle.render()
}

/// A `GET /metrics` router.
///
/// Generic over the caller's own state type via an [`Extension`] layer
/// rather than router state — so it `.merge()`s directly into a service's
/// existing axum app no matter what that app's state is.
///
/// The service is responsible for where this lands: unversioned, outside
/// any auth or CORS layers meant for browser-facing routes.
pub fn route<S>(handle: MetricsHandle) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::new().route("/metrics", get(render)).layer(Extension(handle))
}

/// Stand up a bare listener whose only job is `/metrics`, for a service with
/// no other HTTP surface of its own.
///
/// `addr` should be reachable only from the scrape network — do not publish
/// it to the host unless the service already does so for other reasons.
///
/// # Errors
///
/// Returns [`MetricsError::Bind`] if `addr` cannot be bound, or
/// [`MetricsError::Serve`] if the listener fails while running.
pub async fn serve(addr: SocketAddr, handle: MetricsHandle) -> Result<(), MetricsError> {
	let listener = tokio::net::TcpListener::bind(addr).await.map_err(|source| MetricsError::Bind { addr, source })?;
	tracing::info!(%addr, "metrics listener up");
	axum::serve(listener, route::<()>(handle)).await.map_err(|source| MetricsError::Serve { addr, source })
}

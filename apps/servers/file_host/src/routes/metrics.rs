//! `GET /metrics` — the scrape target `infra/prometheus/prometheus.yml` has
//! declared since before this route existed. See #140.
//!
//! Unversioned and un-CORS'd like `/health`: this is not a browser route,
//! and the ergonomics that protect browser clients (origin checks) are
//! exactly wrong for a scraper. Self-contained on its own state via
//! [`some_metrics::MetricsHandle`] rather than [`crate::AppState`], so it
//! merges into the app router without pulling `/metrics` into anything
//! `/health` and the versioned surface depend on.

use axum::Router;
use some_metrics::MetricsHandle;

pub fn get_metrics<S>(handle: MetricsHandle) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	some_metrics::route(handle)
}

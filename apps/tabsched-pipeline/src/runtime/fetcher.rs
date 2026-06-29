//
// Single responsibility: fetch Vec<TabCapture> from the Axum server.
//
// This is the only place in the pipeline crate that makes outbound HTTP calls
// to the Axum server. All callers receive a pre-filtered slice (extraction_ok
// only) — the filter lives here so process.rs has no knowledge of it.
//
// Error contract:
//   All errors are returned as anyhow::Error. The caller (process.rs) maps
//   them to StageError::Retryable so JetStream NAKs and redelivers on any
//   transient network or server failure.
//
// Retry rationale:
//   The Axum server and pipeline daemon may start in any order. A 503 or
//   connection-refused on the first attempt should not poison the job — the
//   daemon's NAK/redeliver loop handles transient unavailability transparently.
use anyhow::{Context, Result};
use tracing::instrument;
use ws_events::tabsched::TabCapture;

/// Fetch all extraction_ok tabs from the Axum server's GET /tabs endpoint.
///
/// Precondition:  Axum server is reachable at `base_url`.
/// Postcondition: returns only tabs where extraction_ok == true.
///   Empty vec is valid (db is empty or all tabs failed extraction);
///   caller decides whether to treat it as Poison.
///
/// The `http` client is shared across all workers via WorkerCtx — no
/// per-call connection overhead.
#[instrument(skip(http), fields(base_url = %base_url))]
pub async fn fetch_tabs_from_server(http: &reqwest::Client, base_url: &str) -> Result<Vec<TabCapture>> {
	let url = format!("{}/tabs", base_url.trim_end_matches('/'));

	let response = http.get(&url).send().await.with_context(|| format!("GET {} failed", url))?;

	let status = response.status();
	if !status.is_success() {
		anyhow::bail!("GET {} returned {}", url, status);
	}

	let all_tabs: Vec<TabCapture> = response.json().await.with_context(|| format!("failed to deserialize response from {}", url))?;

	// Filter here so process.rs operates on a clean slice with no guards needed.
	let ok_tabs: Vec<TabCapture> = all_tabs.into_iter().filter(|t| t.extraction_ok).collect();

	tracing::info!(total = ok_tabs.len(), "fetched extraction_ok tabs from server");

	Ok(ok_tabs)
}

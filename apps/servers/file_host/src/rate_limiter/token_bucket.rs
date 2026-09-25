use crate::{metrics::rate_limit, net::Peer};
use axum::{body::Body, extract::State, middleware::Next, response::Response};
use some_services::rate_limiter::{PartitionedTokenBucketLimiter, RateLimitError};
use std::sync::Arc;
use tracing::warn;

/// #215 (A1/A2): one decision point, three outcomes.
///
/// (`rate_limit_decisions_total{limiter="http",outcome}`), and a per-client
/// partition keyed the same way `ConnectionGuard` keys WebSocket admission —
/// the peer's [`crate::net::PeerKey`], never the raw address (#372) — so HTTP
/// and WS agree on what "a client" is. Previously this
/// installed a single process-wide bucket sized from `max_request_size`
/// (megabytes of payload, not a request rate) with no counter and no log
/// line on rejection; see `some_services::rate_limiter` for the arithmetic
/// fix and the partitioning this middleware now drives.
///
/// # Errors
///
/// Returns `RateLimitError::RateLimited` (429) when the caller's bucket is
/// empty, or `RateLimitError::ClockError` (500) if the system clock produced
/// an error while refilling.
pub async fn rate_limit_middleware(
	State(limiter): State<Arc<PartitionedTokenBucketLimiter>>,
	Peer { key: peer, .. }: Peer,
	request: axum::http::Request<Body>,
	next: Next,
) -> Result<Response, RateLimitError> {
	match limiter.allow_request(peer.as_str()) {
		Ok(true) => {
			rate_limit::record_decision("allowed");
			Ok(next.run(request).await)
		}
		Ok(false) => {
			warn!(peer = %peer, "rate limit exceeded");
			rate_limit::record_decision("rejected");
			Err(RateLimitError::RateLimited)
		}
		Err(err) => {
			warn!(peer = %peer, error = %err, "rate limiter clock error");
			rate_limit::record_decision("error");
			Err(err)
		}
	}
}

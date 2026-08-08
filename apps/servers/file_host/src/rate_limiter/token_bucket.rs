use crate::metrics::rate_limit;
use axum::{
	body::Body,
	extract::{ConnectInfo, State},
	middleware::Next,
	response::Response,
};
use some_services::rate_limiter::{PartitionedTokenBucketLimiter, RateLimitError};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::warn;

/// #215 (A1/A2): one decision point, three outcomes.
///
/// (`rate_limit_decisions_total{limiter="http",outcome}`), and a per-client
/// partition keyed the same way `ConnectionGuard` keys WebSocket admission —
/// `addr.ip()` — so HTTP and WS agree on what "a client" is. Previously this
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
	ConnectInfo(addr): ConnectInfo<SocketAddr>,
	request: axum::http::Request<Body>,
	next: Next,
) -> Result<Response, RateLimitError> {
	let client_id = addr.ip().to_string();

	match limiter.allow_request(&client_id) {
		Ok(true) => {
			rate_limit::record_decision("allowed");
			Ok(next.run(request).await)
		}
		Ok(false) => {
			warn!(client_id = %client_id, "rate limit exceeded");
			rate_limit::record_decision("rejected");
			Err(RateLimitError::RateLimited)
		}
		Err(err) => {
			warn!(client_id = %client_id, error = %err, "rate limiter clock error");
			rate_limit::record_decision("error");
			Err(err)
		}
	}
}

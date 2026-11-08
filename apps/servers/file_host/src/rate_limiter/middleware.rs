use crate::{
	metrics::rate_limit,
	net::{AdmissionKey, Peer},
};
use axum::{
	body::Body,
	extract::State,
	http::{header::RETRY_AFTER, HeaderValue, StatusCode},
	middleware::Next,
	response::{IntoResponse, Response},
};
use some_services::rate_limiter::{Clock, Decision, KeyedLimiter};
use std::{sync::Arc, time::Duration};
use tracing::warn;

/// The limiter in front of `/api/v1`: one GCRA state per client (the client
/// behind home's reverse proxy when it is a trusted one, see `net::Peer`),
/// never the raw address (#372).
///
/// Keyed by the process-stable [`AdmissionKey`], not the daily `PeerKey`: a
/// key that rotates at midnight UTC would hand every client a fresh burst
/// then. The admission key is only ever a map key here; the log line names
/// the `PeerKey`, as before.
pub type HttpLimiter = KeyedLimiter<AdmissionKey>;

/// One decision point, two outcomes, both counted
/// (`rate_limit_decisions_total{limiter="http",outcome}`).
///
/// A refusal is a `429` with `Retry-After`: the exact wait the limiter
/// computed, rounded up to whole seconds (the header's unit), so a client that
/// waits that long is admitted. The phone's home link stops sending to home
/// for that long on its first `429`, rather than letting every lane try.
///
/// Generic over the clock only so tests can drive it with a manual one.
pub async fn rate_limit_middleware<C: Clock + 'static>(
	State(limiter): State<Arc<KeyedLimiter<AdmissionKey, C>>>,
	Peer { key: peer, admission }: Peer,
	request: axum::http::Request<Body>,
	next: Next,
) -> Response {
	match limiter.check(&admission) {
		Decision::Allowed { .. } => {
			rate_limit::record_decision("allowed");
			next.run(request).await
		}
		Decision::Limited { retry_after } => {
			let seconds = retry_after_seconds(retry_after);
			warn!(peer = %peer, retry_after_secs = seconds, "rate limit exceeded");
			rate_limit::record_decision("rejected");
			(StatusCode::TOO_MANY_REQUESTS, [(RETRY_AFTER, HeaderValue::from(seconds))], "Rate limit exceeded").into_response()
		}
	}
}

/// Whole seconds, rounded up and at least one: rounding down would send the
/// client back before it is admitted.
fn retry_after_seconds(wait: Duration) -> u64 {
	u64::try_from(wait.as_nanos().div_ceil(1_000_000_000)).unwrap_or(u64::MAX).max(1)
}

#[cfg(test)]
mod tests {
	use super::*;
	use axum::{extract::connect_info::MockConnectInfo, http::Request, middleware::from_fn_with_state, routing::get, Router};
	use some_services::rate_limiter::{ManualClock, Quota};
	use std::{net::SocketAddr, num::NonZeroU32};
	use tower::ServiceExt;

	#[test]
	fn retry_after_rounds_up_to_a_whole_second() {
		assert_eq!(retry_after_seconds(Duration::ZERO), 1);
		assert_eq!(retry_after_seconds(Duration::from_nanos(1)), 1);
		assert_eq!(retry_after_seconds(Duration::from_secs(1)), 1);
		assert_eq!(retry_after_seconds(Duration::from_millis(1001)), 2);
		assert_eq!(retry_after_seconds(Duration::from_secs(59)), 59);
	}

	/// A `429` names when to ask again, and asking then is admitted.
	#[tokio::test]
	#[allow(clippy::disallowed_methods)] // a throwaway test router, not a served route
	async fn a_refusal_says_when_to_ask_again() {
		let clock = Arc::new(ManualClock::new());
		let quota = Quota::per_period(NonZeroU32::new(2).unwrap(), Duration::from_secs(60));
		let limiter = Arc::new(KeyedLimiter::new(quota, Arc::clone(&clock)));
		let app = Router::new()
			.route("/", get(|| async { "ok" }))
			.layer(from_fn_with_state(limiter, rate_limit_middleware))
			.layer(MockConnectInfo(SocketAddr::from(([10, 1, 2, 3], 5555))));
		let ask = || app.clone().oneshot(Request::get("/").body(Body::empty()).unwrap());

		assert_eq!(ask().await.unwrap().status(), StatusCode::OK);
		assert_eq!(ask().await.unwrap().status(), StatusCode::OK);
		let refused = ask().await.unwrap();
		assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
		assert_eq!(refused.headers()[RETRY_AFTER], "30", "2 a minute refills one every 30 s");

		clock.advance(Duration::from_secs(30));
		assert_eq!(ask().await.unwrap().status(), StatusCode::OK);
	}
}

//! `http_requests_total{route,method,status}` and a duration histogram on
//! the same dimensions. See #213 (F2).
//!
//! `route` is always the axum-matched path *pattern* (`/api/v1/foo/:id`),
//! never the raw request path — that keeps the label's cardinality bounded
//! to the checked route inventory (`routes::inventory::ROUTES`) rather than
//! unbounded by whatever a client happened to request. A path nothing
//! matched labels as `"unmatched"`; see [`unmatched`], the router's
//! `.fallback(...)` handler that is the only other place this metric is
//! recorded from.

use axum::{
	extract::{MatchedPath, Request},
	http::StatusCode,
	middleware::Next,
	response::{IntoResponse, Response},
};
use metrics::{counter, histogram};
use std::time::Instant;

/// `PrometheusBuilder::set_buckets_for_metric`'s target for [`HTTP_DURATION_METRIC`].
///
/// See `main.rs`'s `some_metrics::install_with` call, which is where this
/// actually takes effect. The exporter's default buckets top out at 10s;
/// `TASK_TIMEOUT_MS` (`config.rs`) defaults to 15,000ms, so a request that
/// times out needs a bucket above that or it falls off the end of the
/// histogram into `+Inf` and p99 becomes unrepresentable for exactly the
/// requests it matters most to see.
pub const HTTP_DURATION_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 15.0, 30.0];

/// Kept as a constant, not a literal at each call site, so the bucket
/// override in `main.rs` and the `histogram!` call below can't drift apart.
pub const HTTP_DURATION_METRIC: &str = "http_request_duration_seconds";

/// `GET /metrics` never counts itself — a scraper hitting its own scrape
/// target every interval would otherwise be the single busiest "route" on
/// every deployment, telling you about Prometheus rather than about the
/// service.
const EXCLUDED_ROUTE: &str = "/metrics";

/// The router's outermost `.layer(middleware::from_fn(...))`. Only runs for
/// requests axum's router actually matched — see [`unmatched`] for the
/// complementary fallback path.
pub async fn track_http_metrics(req: Request, next: Next) -> Response {
	let route = req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_owned());

	match route {
		Some(route) if route == EXCLUDED_ROUTE => next.run(req).await,
		Some(route) => record_and_run(req, next, route).await,
		// A `.layer()` middleware only runs once routing has already
		// resolved (unmatched paths go through `.fallback(unmatched)`
		// instead), so this should be unreachable — falling through to
		// "unmatched" rather than panicking keeps this middleware
		// infallible regardless.
		None => record_and_run(req, next, "unmatched".to_string()).await,
	}
}

async fn record_and_run(req: Request, next: Next, route: String) -> Response {
	let method = req.method().to_string();
	let start = Instant::now();

	let response = next.run(req).await;

	let status = response.status().as_u16().to_string();
	let elapsed = start.elapsed().as_secs_f64();

	counter!("http_requests_total", "route" => route.clone(), "method" => method.clone(), "status" => status).increment(1);
	histogram!(HTTP_DURATION_METRIC, "route" => route, "method" => method).record(elapsed);

	response
}

/// The router's `.fallback(...)`.
///
/// The only other place `http_requests_total` is recorded from — every
/// request no route matched lands here, always labelled `route="unmatched"`
/// regardless of the path actually requested, so a client scanning random
/// paths cannot grow this metric's cardinality.
///
/// `async` despite no `.await`: `Router::fallback`'s `Handler` bound
/// requires it — axum handlers are async by trait, not by choice here.
#[allow(clippy::unused_async)]
pub async fn unmatched(req: Request) -> impl IntoResponse {
	let method = req.method().to_string();
	counter!("http_requests_total", "route" => "unmatched", "method" => method, "status" => "404").increment(1);
	(StatusCode::NOT_FOUND, "not found")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::routes::inventory::{RouteDescriptor, ROUTES};
	use axum::{
		body::Body,
		http::Request as HttpRequest,
		routing::{delete, get, patch, post, put, MethodRouter},
		Router,
	};
	use std::collections::BTreeSet;
	use tower::ServiceExt;

	async fn stub() -> &'static str {
		"ok"
	}

	fn method_router(method: &str) -> MethodRouter {
		match method {
			"GET" => get(stub),
			"POST" => post(stub),
			"PUT" => put(stub),
			"PATCH" => patch(stub),
			"DELETE" => delete(stub),
			other => panic!("unhandled method in ROUTES: {other}"),
		}
	}

	/// #222's acceptance criteria: "the `route` label values are a subset of
	/// the inventory". A bare router (no `AppState`, no handlers with real
	/// bodies — `routes/inventory.rs`'s own tests avoid needing a fully built
	/// `AppState` for exactly this reason) carrying every `ROUTES` path under
	/// the same middleware this app installs, then asserting the
	/// `MatchedPath` this middleware would label with is *exactly*
	/// `route.full_path()` for every declared route. Catches, at compile+test
	/// time rather than in a scrape years from now, a route whose declared
	/// path syntax doesn't actually match what axum resolves (e.g. a stray
	/// `{name}` where axum expects `:name`, or a `/api/v1` nesting mismatch).
	#[tokio::test]
	async fn matched_path_equals_route_inventory_full_path() {
		let versioned: Router = ROUTES.iter().filter(|r| r.versioned).fold(Router::new(), |r, route| r.route(route.path, method_router(route.method)));
		let unversioned: Router = ROUTES.iter().filter(|r| !r.versioned).fold(Router::new(), |r, route| r.route(route.path, method_router(route.method)));
		let app = Router::new().nest(crate::API_V1_BASE_PATH, versioned).merge(unversioned).layer(axum::middleware::from_fn(record_matched_path));

		let inventory_paths: BTreeSet<String> = ROUTES.iter().map(RouteDescriptor::full_path).collect();

		for route in ROUTES {
			// `/ws` upgrades the connection rather than resolving as a plain
			// handler call; a bare GET against it is expected to fail the
			// upgrade, not the routing, so it is exempt from being driven
			// through `oneshot` here.
			if route.path == "/ws" {
				continue;
			}
			let full_path = route.full_path();
			let request = HttpRequest::builder().method(route.method).uri(&full_path).body(Body::empty()).unwrap();
			let response = app.clone().oneshot(request).await.unwrap();
			let matched = response.headers().get("x-test-matched-path").map(|v| v.to_str().unwrap().to_owned());
			assert_eq!(matched.as_deref(), Some(full_path.as_str()), "route {full_path} did not resolve to its own declared path");
			assert!(inventory_paths.contains(&full_path));
		}
	}

	// A middleware that reports the `MatchedPath` it saw back to the test
	// via a response header, so the test above can assert on it without
	// needing the real `metrics` facade recorder installed.
	async fn record_matched_path(req: Request, next: Next) -> Response {
		let matched = req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_owned());
		let mut response = next.run(req).await;
		if let Some(matched) = matched {
			response.headers_mut().insert("x-test-matched-path", matched.parse().unwrap());
		}
		response
	}
}

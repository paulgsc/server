//! The HTTP trace span: a route, never a URL.
//!
//! tower-http's default span (`DefaultMakeSpan`) records the request's whole
//! URI at DEBUG, query string included. A query string and a free-text path
//! segment are whatever the caller typed, so a deployment that turns on
//! `tower_http=debug` (or `RUST_LOG=debug`) would write them to the log and
//! export them through the OpenTelemetry layer.
//!
//! [`RouteSpan`] records the method and the route *template* axum matched
//! (`/api/v1/tabs/:tab_id`), or one fixed word for a request no route matched
//! (the same bounded label `metrics::http` uses), and nothing else from the
//! request. docs/identity.md, invariant 13.

use axum::{extract::MatchedPath, http::Request};
use tower_http::trace::MakeSpan;
use tracing::{Level, Span};

/// The route label for a request no route matched. Fixed, so an unbounded set
/// of garbage paths can neither grow a span's cardinality nor reach a log.
pub const UNMATCHED_ROUTE: &str = "unmatched";

/// Builds the `request` span from the method and the matched route template.
///
/// Level `DEBUG`, as tower-http's own default is, so nothing new appears at
/// the default `info` filter.
#[derive(Debug, Clone, Copy, Default)]
pub struct RouteSpan;

impl<B> MakeSpan<B> for RouteSpan {
	fn make_span(&mut self, request: &Request<B>) -> Span {
		let route = request.extensions().get::<MatchedPath>().map_or(UNMATCHED_ROUTE, MatchedPath::as_str);
		tracing::span!(Level::DEBUG, "request", method = %request.method(), route)
	}
}

#[cfg(test)]
mod tests {
	use super::{RouteSpan, UNMATCHED_ROUTE};
	use crate::privacy::capture;
	use axum::{
		body::Body,
		http::{Request, StatusCode},
		routing::post,
		Router,
	};
	use tower::ServiceExt;
	use tower_http::trace::TraceLayer;

	/// What a caller typed, and must not see written down.
	const QUERY: &str = "email=jane.doe@example.test&token=SECRET-QUERY-MARKER";
	const SEGMENT: &str = "SECRET-SEGMENT-MARKER";
	const MARKERS: [&str; 3] = ["jane.doe@example.test", "SECRET-QUERY-MARKER", SEGMENT];

	/// A router shaped like `main`'s: a matched route, an unmatched fallback,
	/// and the layer added after both so `MatchedPath` is already set.
	#[allow(clippy::disallowed_methods)] // a throwaway test router, not a served route
	fn app<M>(make_span: M) -> Router
	where
		M: tower_http::trace::MakeSpan<Body> + Clone + Send + Sync + 'static,
	{
		Router::new()
			.route("/api/v1/tabs/:tab_id", post(|| async { "ok" }))
			.fallback(|| async { StatusCode::NOT_FOUND })
			.layer(TraceLayer::new_for_http().make_span_with(make_span))
	}

	/// A path a caller made up, with its query string. Built by concatenation:
	/// `format!` is a disallowed macro here (clippy.toml).
	fn typed(path: &str) -> String {
		String::from(path) + SEGMENT + "?" + QUERY
	}

	async fn send(app: &Router, method: &str, path: &str) -> StatusCode {
		let request = Request::builder().method(method).uri(path).body(Body::empty()).unwrap();
		app.clone().oneshot(request).await.unwrap().status()
	}

	/// docs/identity.md, invariant 13.
	#[tokio::test]
	async fn a_request_span_names_the_route_and_never_the_query_or_a_path_segment() {
		let (captured, _guard) = capture();
		let app = app(RouteSpan);

		assert_eq!(send(&app, "POST", &typed("/api/v1/tabs/")).await, StatusCode::OK);
		assert_eq!(send(&app, "GET", &typed("/no/such/")).await, StatusCode::NOT_FOUND);

		let lines = captured.lines();
		// Without these the test passes vacuously: the spans must exist, and
		// must carry the template and the fixed word that replace the URL.
		for expected in ["route=/api/v1/tabs/:tab_id".to_owned(), String::from("route=") + UNMATCHED_ROUTE, "method=POST".to_owned()] {
			assert!(lines.iter().any(|line| line == &expected), "no span field {expected:?}: {lines:#?}");
		}
		for line in &lines {
			for marker in MARKERS {
				assert!(!line.contains(marker), "{marker} reached a log line: {line}");
			}
		}
	}

	/// The control for the test above: tower-http's default span is what
	/// `main` used before, and it does record the URI. If this stops
	/// failing-to-hide the markers, the test above no longer proves anything.
	#[tokio::test]
	async fn the_default_span_this_replaces_records_the_whole_uri() {
		let (captured, _guard) = capture();
		let app = app(tower_http::trace::DefaultMakeSpan::new());

		assert_eq!(send(&app, "POST", &typed("/api/v1/tabs/")).await, StatusCode::OK);

		let lines = captured.lines();
		assert!(
			lines.iter().any(|line| line.contains("SECRET-QUERY-MARKER") && line.contains(SEGMENT)),
			"tower-http's default span no longer records the URI, so the replacement is untested: {lines:#?}"
		);
	}
}

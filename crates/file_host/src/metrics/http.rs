use axum::{
	body::Body,
	http::{Request, Response, StatusCode},
	middleware::Next,
};
use lazy_static::lazy_static;
use prometheus::{register_histogram_vec, register_int_counter_vec, Encoder, HistogramVec, IntCounterVec, TextEncoder};
use std::time::Instant;

lazy_static! {
	static ref HTTP_REQUESTS_TOTAL: IntCounterVec =
		register_int_counter_vec!("http_requests_total", "Total number of HTTP requests", &["method", "route", "status"]).expect("Failed to register HTTP_REQUESTS_TOTAL");
	static ref HTTP_REQUEST_DURATION: HistogramVec =
		register_histogram_vec!("http_request_duration_seconds", "HTTP request duration in seconds", &["method", "route"]).expect("Failed to register HTTP_REQUEST_DURATION");
	pub static ref OPERATION_DURATION: HistogramVec = register_histogram_vec!(
		"operation_duration_seconds",
		"Duration of specific operations in seconds",
		&["handler", "operation", "cache_hit"]
	)
	.expect("Failed to register OPERATION_DURATION");
}

/// Middleware for Prometheus metrics collection
pub async fn metrics_middleware(req: Request<Body>, next: Next) -> Response<Body> {
	let method = req.method().to_string();
	let path = normalize_path(req.uri().path());

	let start = Instant::now();
	let response = next.run(req).await;
	let duration = start.elapsed().as_secs_f64();

	let status = response.status().as_u16().to_string();

	HTTP_REQUESTS_TOTAL.with_label_values(&[&method, &path, &status]).inc();
	HTTP_REQUEST_DURATION.with_label_values(&[&method, &path]).observe(duration);

	response
}

/// Normalize the route path for consistent labeling
fn normalize_path(path: &str) -> String {
	path.trim_end_matches('/').split('?').next().unwrap_or("/").to_string()
}

/// Prometheus metrics handler
pub async fn metrics_handler() -> Result<String, StatusCode> {
	let encoder = TextEncoder::new();
	let metric_families = prometheus::gather();
	let mut buffer = Vec::new();

	if encoder.encode(&metric_families, &mut buffer).is_err() {
		return Err(StatusCode::INTERNAL_SERVER_ERROR);
	}

	String::from_utf8(buffer).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[macro_export]
macro_rules! timed_operation {
	($handler:expr, $operation:expr, $cache_hit:expr, $body:block) => {{
		let start = std::time::Instant::now();
		let result = $body;
		let duration = start.elapsed().as_secs_f64();

		OPERATION_DURATION.with_label_values(&[$handler, $operation, &$cache_hit.to_string()]).observe(duration);

		tracing::info!(
			handler = $handler,
			operation = $operation,
			duration_ms = duration * 1000.0,
			cache_hit = $cache_hit,
			"Operation completed"
		);

		result
	}};
}

use super::OtelGuard;
use axum::{extract::State, http::StatusCode, response::IntoResponse};
use std::sync::Arc;

pub async fn metrics_handler(State(otel_guard): State<Arc<OtelGuard>>) -> impl IntoResponse {
	match otel_guard.metrics() {
		Ok(metrics) => (StatusCode::OK, [("Content-Type", "text/plain; version=0.0.4")], metrics).into_response(),
		Err(e) => {
			tracing::error!("Failed to gather metrics: {}", e);
			StatusCode::INTERNAL_SERVER_ERROR("Failed to gather metrics").into_response()
		}
	}
}

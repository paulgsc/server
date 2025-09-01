use axum::{http::StatusCode, response::Json};
use serde::Serialize;
use tracing::instrument;

#[derive(Serialize)]
pub struct HealthResponse {
	status: &'static str,
	version: &'static str,
}

#[axum::debug_handler]
#[instrument(name = "health")]
pub async fn health() -> (StatusCode, Json<HealthResponse>) {
	let response = HealthResponse {
		status: "healthy",
		version: env!("CARGO_PKG_VERSION"), // pulled from Cargo.toml at compile time
	};

	(StatusCode::OK, Json(response))
}

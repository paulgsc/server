use axum::http::StatusCode;
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "health")]
pub async fn health() -> StatusCode {
	StatusCode::OK
}

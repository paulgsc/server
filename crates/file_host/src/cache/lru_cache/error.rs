use crate::GSheetDeriveError;
use axum::{
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use thiserror::Error;

/// Cache-specific errors
#[derive(Error, Debug)]
pub enum DedupError {
	#[error("Serialization error: {0}")]
	SerializationError(#[from] serde_json::Error),

	#[error("Cache store error: {0}")]
	StoreError(#[from] crate::cache::CacheError),

	#[error("Operation error: {0}")]
	OperationError(String),

	#[error("Type mismatch: {0}")]
	TypeMismatch(String),

	#[error("Sheet Derive error: {0}")]
	GSheetError(#[from] GSheetDeriveError),

	#[error("Sheet error: {0}")]
	SheetError(#[from] sdk::SheetError),

	#[error("Expected exactly one key-value pair, found none")]
	UnexpectedSinglePair,

	#[error("Polars error: {0}")]
	PolarsError(#[from] polars::error::PolarsError),

	#[error("Drive error: {0}")]
	DriveError(#[from] sdk::DriveError),

	#[error("GithubAPI error: {0}")]
	GitHubAPIError(#[from] sdk::GitHubError),

	#[error("Audio Service error: {0}")]
	AudioError(#[from] crate::AudioServiceError),

	#[error("Database Error: {0}")]
	SqliteError(#[from] sqlx::Error),
}
impl IntoResponse for DedupError {
	fn into_response(self) -> Response {
		let (status, message) = match self {
			DedupError::OperationError(msg) if msg.contains("not found") => (StatusCode::NOT_FOUND, msg),
			DedupError::OperationError(msg) if msg.contains("Validation error") => (StatusCode::BAD_REQUEST, msg),
			DedupError::SerializationError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Serialization error".to_string()),
			DedupError::StoreError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Cache store error".to_string()),
			DedupError::SqliteError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Database error".to_string()),
			_ => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string()),
		};

		(status, Json(serde_json::json!({"error": message}))).into_response()
	}
}

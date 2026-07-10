use axum::{
	body::Body,
	http::{header::WWW_AUTHENTICATE, HeaderValue, Response, StatusCode},
	response::IntoResponse,
	Json,
};
use some_cache::DedupCacheError;
use std::{borrow::Cow, collections::HashMap};

// ── GSheetDeriveError ────────────────────────────────────────────────────────

#[derive(thiserror::Error, Debug)]
pub enum GSheetDeriveError {
	#[error("Missing required field {0} at column {1}")]
	MissingRequiredField(String, String),

	#[error("Failed to parse field {0} at column {1}: {2}")]
	ParseError(String, String, String),

	#[error("Column {0} not found in header")]
	ColumnNotFound(String),

	#[error("Missing header row")]
	MissingHeader,
}

// ── FileHostError ────────────────────────────────────────────────────────────

#[derive(thiserror::Error, Debug)]
pub enum FileHostError {
	// ---- auth / routing ----
	#[error("authentication required")]
	Unauthorized,

	#[error("user may not perform that action")]
	Forbidden,

	#[error("request path not found")]
	NotFound,

	// ---- request shape ----
	#[error("invalid data schema")]
	InvalidData,

	#[error("invalid mime type: {0}")]
	InvalidMimeType(String),

	#[error("invalid encoded date: {0}")]
	InvalidEncodedDate(String),

	#[error("error in request body")]
	UnprocessableEntity { errors: HashMap<Cow<'static, str>, Vec<Cow<'static, str>>> },

	#[error("maximum record limit exceeded")]
	MaxRecordLimitExceeded,

	// ---- transparent from-conversions ----
	#[error("serialization error: {0}")]
	NonSerializableData(#[from] serde_json::Error),

	#[error("integer conversion failed: {0}")]
	IntegerConversionError(#[from] std::num::TryFromIntError),

	#[error("response build error: {0}")]
	ResponseBuildError(#[from] axum::http::Error),

	#[error("I/O error: {0}")]
	IoError(#[from] std::io::Error),

	#[error("tower error: {0}")]
	TowerError(#[from] tower::BoxError),

	#[error("NATS transport error: {0}")]
	NatsTransportError(#[from] some_transport::TransportError),

	#[error("audio fetch error: {0}")]
	AudioFetchError(#[from] crate::AudioServiceError),

	#[error("broadcast error: {0}")]
	BroadcastError(#[from] crate::websocket::broadcast::BroadcastError),

	#[error("sheet derive error: {0}")]
	GSheetError(#[from] GSheetDeriveError),

	// ---- upstream SDK errors (collapsed to anyhow) ----
	#[error("upstream error: {0}")]
	Upstream(#[from] anyhow::Error),

	// ---- cache layer ----
	#[error("cache error: {0}")]
	Cache(DedupCacheError),

	// ---- former DedupError variants ----
	#[error("database error: {0}")]
	Sqlite(#[from] sqlx::Error),

	#[error("polars error: {0}")]
	Polars(#[from] polars::error::PolarsError),

	#[error("operation error: {0}")]
	OperationError(String),

	#[error("expected exactly one key-value pair")]
	UnexpectedSinglePair,

	// ---- timeout / load ----
	#[error("request timeout")]
	RequestTimeout,

	#[error("service temporarily overloaded")]
	ServiceOverloaded,
}

impl FileHostError {
	pub fn unprocessable_entity<K, V>(errors: impl IntoIterator<Item = (K, V)>) -> Self
	where
		K: Into<Cow<'static, str>>,
		V: Into<Cow<'static, str>>,
	{
		let mut map: HashMap<Cow<'static, str>, Vec<Cow<'static, str>>> = HashMap::new();
		for (k, v) in errors {
			map.entry(k.into()).or_default().push(v.into());
		}
		Self::UnprocessableEntity { errors: map }
	}

	pub fn upstream<E: std::error::Error + Send + Sync + 'static>(e: E) -> Self {
		Self::Upstream(anyhow::anyhow!(e))
	}
}

// Manual From for DedupCacheError because the cache variant needs HTTP-aware
// decomposition rather than a straight wrap.
impl From<DedupCacheError> for FileHostError {
	fn from(e: DedupCacheError) -> Self {
		match e {
			DedupCacheError::NotFound => Self::NotFound,
			DedupCacheError::OperationError(msg) => Self::unprocessable_entity([("cache", msg)]),
			DedupCacheError::SerializationError(e) => Self::Upstream(anyhow::anyhow!(e)),
			DedupCacheError::StoreError(e) => Self::Upstream(anyhow::anyhow!(e)),
			DedupCacheError::TypeMismatch(e) => Self::Upstream(anyhow::anyhow!(e)),
		}
	}
}

#[derive(serde::Serialize)]
struct ErrorBody {
	code: &'static str,
	message: &'static str,
	#[serde(skip_serializing_if = "Option::is_none")]
	details: Option<HashMap<Cow<'static, str>, Vec<Cow<'static, str>>>>,
}

#[derive(serde::Serialize)]
struct ErrorEnvelope {
	error: ErrorBody,
}

impl FileHostError {
	fn status_code(&self) -> StatusCode {
		match self {
			Self::UnprocessableEntity { .. } => StatusCode::UNPROCESSABLE_ENTITY,
			Self::Unauthorized => StatusCode::UNAUTHORIZED,
			Self::Forbidden => StatusCode::FORBIDDEN,
			Self::NotFound => StatusCode::NOT_FOUND,
			Self::InvalidData | Self::InvalidEncodedDate(_) => StatusCode::FORBIDDEN,
			Self::InvalidMimeType(_) | Self::MaxRecordLimitExceeded | Self::IntegerConversionError(_) | Self::OperationError(_) | Self::UnexpectedSinglePair => {
				StatusCode::BAD_REQUEST
			}
			Self::RequestTimeout => StatusCode::REQUEST_TIMEOUT,
			Self::ServiceOverloaded => StatusCode::SERVICE_UNAVAILABLE,
			Self::AudioFetchError(_) => StatusCode::BAD_REQUEST,
			Self::Cache(e) => match e {
				DedupCacheError::NotFound => StatusCode::NOT_FOUND,
				DedupCacheError::OperationError(_) => StatusCode::BAD_REQUEST,
				_ => StatusCode::INTERNAL_SERVER_ERROR,
			},
			_ => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}

	fn code(&self) -> &'static str {
		match self {
			Self::Unauthorized => "unauthorized",
			Self::Forbidden => "forbidden",
			Self::NotFound => "not_found",
			Self::InvalidData => "invalid_data",
			Self::InvalidMimeType(_) => "invalid_mime_type",
			Self::InvalidEncodedDate(_) => "invalid_encoded_date",
			Self::UnprocessableEntity { .. } => "unprocessable_entity",
			Self::MaxRecordLimitExceeded => "max_record_limit_exceeded",
			Self::NonSerializableData(_) => "serialization_error",
			Self::IntegerConversionError(_) => "integer_conversion_error",
			Self::ResponseBuildError(_) => "response_build_error",
			Self::IoError(_) => "io_error",
			Self::TowerError(_) => "tower_error",
			Self::NatsTransportError(_) => "nats_transport_error",
			Self::AudioFetchError(_) => "audio_fetch_error",
			Self::BroadcastError(_) => "broadcast_error",
			Self::GSheetError(_) => "sheet_derive_error",
			Self::Upstream(_) => "upstream_error",
			Self::Cache(_) => "cache_error",
			Self::Sqlite(_) => "database_error",
			Self::Polars(_) => "polars_error",
			Self::OperationError(_) => "operation_error",
			Self::UnexpectedSinglePair => "unexpected_single_pair",
			Self::RequestTimeout => "request_timeout",
			Self::ServiceOverloaded => "service_overloaded",
		}
	}

	// Client-facing message. Kept generic for anything that might carry
	// internal details (DB/IO/upstream errors) so those never leak into the response body.
	fn message(&self) -> &'static str {
		match self {
			Self::Unauthorized => "authentication required",
			Self::Forbidden => "user may not perform that action",
			Self::NotFound => "request path not found",
			Self::InvalidData => "invalid data schema",
			Self::InvalidMimeType(_) => "invalid mime type",
			Self::InvalidEncodedDate(_) => "invalid encoded date",
			Self::UnprocessableEntity { .. } => "error in request body",
			Self::MaxRecordLimitExceeded => "maximum record limit exceeded",
			Self::RequestTimeout => "request timeout",
			Self::ServiceOverloaded => "service temporarily overloaded",
			Self::AudioFetchError(_) => "audio fetch error",
			_ => "internal server error",
		}
	}
}

impl IntoResponse for FileHostError {
	fn into_response(self) -> Response<Body> {
		tracing::error!(error = ?self, "request failed");

		let status = self.status_code();
		let code = self.code();
		let message = self.message();
		let is_unauthorized = matches!(&self, Self::Unauthorized);
		let details = match self {
			Self::UnprocessableEntity { errors } => Some(errors),
			_ => None,
		};

		let mut response = (status, Json(ErrorEnvelope { error: ErrorBody { code, message, details } })).into_response();

		if is_unauthorized {
			response.headers_mut().insert(WWW_AUTHENTICATE, HeaderValue::from_static("Token"));
		}

		response
	}
}

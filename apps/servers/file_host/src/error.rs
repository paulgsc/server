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

impl IntoResponse for FileHostError {
	fn into_response(self) -> Response<Body> {
		tracing::error!(error = ?self, "request failed");

		match self {
			// ---- structured bodies ----
			Self::UnprocessableEntity { errors } => {
				#[derive(serde::Serialize)]
				struct Body {
					errors: HashMap<Cow<'static, str>, Vec<Cow<'static, str>>>,
				}
				(StatusCode::UNPROCESSABLE_ENTITY, Json(Body { errors })).into_response()
			}

			Self::Unauthorized => (StatusCode::UNAUTHORIZED, [(WWW_AUTHENTICATE, HeaderValue::from_static("Token"))], "authentication required").into_response(),

			Self::Upstream(ref e) => {
				log::error!("upstream error: {:?}", e);
				(StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
			}

			// ---- status-only arms ----
			other => {
				let status = match &other {
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
				};
				(status, "internal server error").into_response()
			}
		}
	}
}

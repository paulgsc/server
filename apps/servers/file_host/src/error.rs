//! `FileHostError`, the one error every handler returns, and its one answer:
//! the `IntoResponse` below picks the status, `code` and client-facing message.
//!
//! **ER1: an error on a request is answered as that error, and a failure
//! nobody can be answered about is counted.**
//!
//! - _Claim:_ every `FileHostError` variant is answered with a status and a
//!   message chosen for that variant (and, for `Cache`, for each
//!   `DedupCacheError` inside it); a handler that fails returns its failure
//!   rather than a success status; and in `crate::websocket`, a client
//!   command that cannot be broadcast is counted and answered to that client,
//!   and a connection's removal leaves no `Result` for a caller to drop.
//! - _Family:_ every variant, today's and any added later; every handler under
//!   `crate::handlers`, including `POST /api/v1/utter` when its broadcast
//!   fails; every path that removes a WebSocket connection (close frame,
//!   stale reaper, forwarder end, socket drop, failed setup, shutdown).
//! - _Proof:_ `status_code`, `code` and `message` match every variant with no
//!   `_ =>` arm, so a new variant does not compile until it is mapped, and
//!   `clippy::wildcard_enum_match_arm` (on for this module and `handlers`)
//!   fails a wildcard added back. `clippy::let_underscore_must_use` (on for
//!   `handlers`) fails a dropped `Result`. `WebSocketFsm::remove_connection`
//!   returns `()`, counting the one failure inside it. Tests answer a failed
//!   broadcast through a disconnected transport: `handlers::utterance`'s, and
//!   `websocket`'s `a_command_that_cannot_be_broadcast_is_counted_and_answered`.
#![warn(clippy::let_underscore_must_use, clippy::wildcard_enum_match_arm)]

use axum::{
	body::Body,
	http::{header::WWW_AUTHENTICATE, HeaderValue, Response, StatusCode},
	response::IntoResponse,
	Json,
};
use some_cache::DedupCacheError;
use std::{borrow::Cow, collections::HashMap};

// ── FileHostError ────────────────────────────────────────────────────────────

#[derive(thiserror::Error, Debug)]
pub enum FileHostError {
	// ---- auth / routing ----
	#[error("authentication required")]
	Unauthorized,

	#[error("user may not perform that action")]
	Forbidden,

	// Account creation is closed: the box already has its owner's account
	// and `AUTH_OPEN_SIGNUP` is off (docs/identity.md, "Who can make an
	// account"). A 403 of its own code, so the client can say "sign in"
	// rather than "something went wrong".
	#[error("this server already has its owner's account")]
	SignupClosed,

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

	// The request is well-formed but contradicts something already recorded —
	// a replayed `POST /outcomes` for a block whose stored outcome differs.
	// Distinct from 422: nothing is wrong with the body on its own.
	#[error("conflicts with an existing record: {0}")]
	Conflict(&'static str),

	// A paired phone wrote a kind of record its person stopped sending home
	// (`handlers::sharing::refusing`). A 409 of its own code, so the phone
	// can tell it from a `Conflict` on the same route.
	#[error("this phone stopped sending it to home")]
	Stopped,

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

	#[error("broadcast error: {0}")]
	BroadcastError(#[from] crate::websocket::broadcast::BroadcastError),

	// ---- upstream errors (collapsed to anyhow) ----
	#[error("upstream error: {0}")]
	Upstream(#[from] anyhow::Error),

	// ---- cache layer ----
	#[error("cache error: {0}")]
	Cache(DedupCacheError),

	// ---- former DedupError variants ----
	#[error("database error: {0}")]
	Sqlite(#[from] sqlx::Error),

	#[error("operation error: {0}")]
	OperationError(String),

	#[error("expected exactly one key-value pair")]
	UnexpectedSinglePair,

	// ---- timeout / load ----
	#[error("request timeout")]
	RequestTimeout,

	#[error("service temporarily overloaded")]
	ServiceOverloaded,

	// A feature this build supports but this deployment has not configured —
	// the study nudge without its VAPID keys, for instance. Distinct from an
	// error on purpose: nothing is broken, and a client that gets a 500 here
	// will report an outage rather than a missing environment variable.
	#[error("feature not configured: {0}")]
	FeatureNotConfigured(&'static str),
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
	const fn status_code(&self) -> StatusCode {
		match self {
			Self::UnprocessableEntity { .. } => StatusCode::UNPROCESSABLE_ENTITY,
			Self::Unauthorized => StatusCode::UNAUTHORIZED,
			Self::Forbidden | Self::SignupClosed | Self::InvalidData | Self::InvalidEncodedDate(_) => StatusCode::FORBIDDEN,
			Self::NotFound => StatusCode::NOT_FOUND,
			Self::Conflict(_) | Self::Stopped => StatusCode::CONFLICT,
			Self::InvalidMimeType(_) | Self::MaxRecordLimitExceeded | Self::IntegerConversionError(_) | Self::UnexpectedSinglePair => StatusCode::BAD_REQUEST,
			Self::RequestTimeout => StatusCode::REQUEST_TIMEOUT,
			Self::ServiceOverloaded | Self::FeatureNotConfigured(_) => StatusCode::SERVICE_UNAVAILABLE,
			Self::Cache(e) => match e {
				DedupCacheError::NotFound => StatusCode::NOT_FOUND,
				DedupCacheError::OperationError(_) => StatusCode::BAD_REQUEST,
				DedupCacheError::SerializationError(_) | DedupCacheError::StoreError(_) | DedupCacheError::TypeMismatch(_) => StatusCode::INTERNAL_SERVER_ERROR,
			},
			Self::NonSerializableData(_)
			| Self::ResponseBuildError(_)
			| Self::IoError(_)
			| Self::TowerError(_)
			| Self::NatsTransportError(_)
			| Self::BroadcastError(_)
			| Self::Upstream(_)
			| Self::Sqlite(_)
			| Self::OperationError(_) => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}

	const fn code(&self) -> &'static str {
		match self {
			Self::Unauthorized => "unauthorized",
			Self::Forbidden => "forbidden",
			Self::SignupClosed => "signup_closed",
			Self::NotFound => "not_found",
			Self::InvalidData => "invalid_data",
			Self::InvalidMimeType(_) => "invalid_mime_type",
			Self::InvalidEncodedDate(_) => "invalid_encoded_date",
			Self::UnprocessableEntity { .. } => "unprocessable_entity",
			Self::MaxRecordLimitExceeded => "max_record_limit_exceeded",
			Self::Conflict(_) => "conflict",
			Self::Stopped => "sharing_stopped",
			Self::NonSerializableData(_) => "serialization_error",
			Self::IntegerConversionError(_) => "integer_conversion_error",
			Self::ResponseBuildError(_) => "response_build_error",
			Self::IoError(_) => "io_error",
			Self::TowerError(_) => "tower_error",
			Self::NatsTransportError(_) => "nats_transport_error",
			Self::BroadcastError(_) => "broadcast_error",
			Self::Upstream(_) => "upstream_error",
			Self::Cache(_) => "cache_error",
			Self::Sqlite(_) => "database_error",
			Self::OperationError(_) => "operation_error",
			Self::UnexpectedSinglePair => "unexpected_single_pair",
			Self::RequestTimeout => "request_timeout",
			Self::ServiceOverloaded => "service_overloaded",
			Self::FeatureNotConfigured(_) => "feature_not_configured",
		}
	}

	// Client-facing message. Kept generic for anything that might carry
	// internal details (DB/IO/upstream errors) so those never leak into the response body.
	// A 4xx says what was wrong with the request, never "internal server error".
	const fn message(&self) -> &'static str {
		match self {
			Self::Unauthorized => "authentication required",
			Self::Forbidden => "user may not perform that action",
			Self::SignupClosed => "this server already has its owner's account; sign in with its passkey",
			Self::NotFound => "request path not found",
			Self::InvalidData => "invalid data schema",
			Self::InvalidMimeType(_) => "invalid mime type",
			Self::InvalidEncodedDate(_) => "invalid encoded date",
			Self::UnprocessableEntity { .. } => "error in request body",
			Self::MaxRecordLimitExceeded => "maximum record limit exceeded",
			Self::IntegerConversionError(_) => "integer out of range",
			Self::UnexpectedSinglePair => "expected exactly one key-value pair",
			Self::Conflict(reason) => reason,
			Self::Stopped => "this phone stopped sending it to home",
			Self::RequestTimeout => "request timeout",
			Self::ServiceOverloaded => "service temporarily overloaded",
			Self::FeatureNotConfigured(_) => "feature not configured on this deployment",
			Self::Cache(e) => match e {
				DedupCacheError::NotFound => "not found",
				DedupCacheError::OperationError(_) => "cache operation refused",
				DedupCacheError::SerializationError(_) | DedupCacheError::StoreError(_) | DedupCacheError::TypeMismatch(_) => "internal server error",
			},
			Self::NonSerializableData(_)
			| Self::ResponseBuildError(_)
			| Self::IoError(_)
			| Self::TowerError(_)
			| Self::NatsTransportError(_)
			| Self::BroadcastError(_)
			| Self::Upstream(_)
			| Self::Sqlite(_)
			| Self::OperationError(_) => "internal server error",
		}
	}
}

impl IntoResponse for FileHostError {
	fn into_response(self) -> Response<Body> {
		let status = self.status_code();

		// 5xx is this service's own fault and belongs in the ERROR-level
		// signal `ErrorEventMetricsLayer` (metrics/observability.rs) mirrors
		// into `tracing_events_total` for alerting; a 4xx is the client's
		// request, not an operational fault — a mistyped token or a bad
		// mime-type header shouldn't page anyone, so it stays at WARN and
		// out of that metric.
		if status.is_server_error() {
			tracing::error!(error = ?self, "request failed");
		} else {
			tracing::warn!(error = ?self, "request rejected");
		}

		let code = self.code();
		let message = self.message();
		let is_unauthorized = matches!(&self, Self::Unauthorized);
		// Only `UnprocessableEntity` carries details; every other variant's
		// answer is its `code` and `message` alone.
		let details = if let Self::UnprocessableEntity { errors } = self { Some(errors) } else { None };

		let mut response = (
			status,
			Json(ErrorEnvelope {
				error: ErrorBody { code, message, details },
			}),
		)
			.into_response();

		if is_unauthorized {
			response.headers_mut().insert(WWW_AUTHENTICATE, HeaderValue::from_static("Token"));
		}

		response
	}
}

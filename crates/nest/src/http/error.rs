use axum::body::Body;
use axum::http::header::WWW_AUTHENTICATE;
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use sqlx::error::DatabaseError;
use sqlx::migrate::MigrateError;
use std::borrow::Cow;
use std::collections::HashMap;

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("authentication required")]
	Unauthorized,

	#[error("user may not perform that action")]
	Forbidden,

	#[error("request path not found")]
	NotFound,

	#[error("Invalid Data Schema pased")]
	InvalidData,

	#[error("error in the request body")]
	UnprocessableEntity { errors: HashMap<Cow<'static, str>, Vec<Cow<'static, str>>> },

	#[error("an error occurred with the database")]
	Sqlx(#[from] sqlx::Error),

	#[error("an internal server error occurred")]
	Anyhow(#[from] anyhow::Error),

	#[error("maximum record limit exceeded")]
	MaxRecordLimitExceeded,

	#[error("integer conversion failed: {0}")]
	IntegerConversionError(#[from] std::num::TryFromIntError),

	#[error("Encoded Date Conversion failed: {0}")]
	InvalidEncodedDate(String),

	#[error("migration error occurred")]
	Migrate(#[from] MigrateError),
}

impl Error {
	pub fn unprocessable_entity<K, V>(errors: impl IntoIterator<Item = (K, V)>) -> Self
	where
		K: Into<Cow<'static, str>>,
		V: Into<Cow<'static, str>>,
	{
		let mut error_map = HashMap::new();

		for (key, val) in errors {
			error_map.entry(key.into()).or_insert_with(Vec::new).push(val.into());
		}

		Self::UnprocessableEntity { errors: error_map }
	}

	const fn status_code(&self) -> StatusCode {
		match self {
			Self::Unauthorized => StatusCode::UNAUTHORIZED,
			Self::Forbidden => StatusCode::FORBIDDEN,
			Self::NotFound => StatusCode::NOT_FOUND,
			Self::InvalidData => StatusCode::FORBIDDEN,
			Self::InvalidEncodedDate(_) => StatusCode::FORBIDDEN,
			Self::UnprocessableEntity { .. } => StatusCode::UNPROCESSABLE_ENTITY,
			Self::Sqlx(_) | Self::Anyhow(_) => StatusCode::INTERNAL_SERVER_ERROR,
			Self::MaxRecordLimitExceeded => StatusCode::BAD_REQUEST,
			Self::IntegerConversionError(_) => StatusCode::BAD_REQUEST,

			Self::Migrate(_) => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}
}

impl IntoResponse for Error {
	fn into_response(self) -> Response<Body> {
		match self {
			Self::UnprocessableEntity { errors } => {
				#[derive(serde::Serialize)]
				struct Errors {
					errors: HashMap<Cow<'static, str>, Vec<Cow<'static, str>>>,
				}

				return (StatusCode::UNPROCESSABLE_ENTITY, Json(Errors { errors })).into_response();
			}
			Self::Unauthorized => {
				return (
					self.status_code(),
					[(WWW_AUTHENTICATE, HeaderValue::from_static("Token"))].into_iter().collect::<HeaderMap>(),
					self.to_string(),
				)
					.into_response();
			}

			Self::Sqlx(ref e) => {
				log::error!("SQLx error: {:?}", e);
			}

			Self::Anyhow(ref e) => {
				log::error!("Generic error: {:?}", e);
			}

			Self::MaxRecordLimitExceeded => {
				return (StatusCode::BAD_REQUEST, self.to_string()).into_response();
			}

			Self::Migrate(ref e) => {
				log::error!("Migration error: {:?}", e);
			}

			// Other errors get mapped normally.
			_ => (),
		}

		(self.status_code(), self.to_string()).into_response()
	}
}

pub trait ResultExt<T> {
	fn on_constraint(self, name: &str, f: impl FnOnce(Box<dyn DatabaseError>) -> Error) -> Result<T, Error>;
}

impl<T, E> ResultExt<T> for Result<T, E>
where
	E: Into<Error>,
{
	fn on_constraint(self, name: &str, map_err: impl FnOnce(Box<dyn DatabaseError>) -> Error) -> Result<T, Error> {
		self.map_err(|e| match e.into() {
			Error::Sqlx(sqlx::Error::Database(dbe)) if dbe.constraint() == Some(name) => map_err(dbe),
			e => e,
		})
	}
}

use axum::body::Body;
use axum::http::Response;
use axum::response::IntoResponse;
use nest::http::Error as NestError;
use nfl_play_parser::error::PlayTypeError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NflServerError {
	#[error(transparent)]
	NestError(#[from] NestError), // Reuse nest's error

	#[error("an error occurred with the play type")]
	PlayTypeError(#[from] PlayTypeError),

	#[error("Invalid game status: {0}")]
	GameStatusParseError(String),
}

impl IntoResponse for NflServerError {
	fn into_response(self) -> Response<Body> {
		match self {
			Self::NestError(nest_err) => nest_err.into_response(), // Delegate to nest's implementation
			Self::PlayTypeError(err) => {
				// Handle PlayTypeError specifically if needed, otherwise delegate
				Response::builder().status(400).body(format!("Play type error: {}", err).into()).unwrap()
			}
			Self::GameStatusParseError(err) => Response::builder().status(500).body(format!("game status parse error: {}", err).into()).unwrap(),
		}
	}
}

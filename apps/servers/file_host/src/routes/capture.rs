use crate::{
	handlers::capture::{delete_capture, get_capture, list_captures, post_capture},
	AppState,
};
use axum::{
	extract::FromRef,
	routing::{delete, get, post},
	Router,
};

pub fn capture_routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Router::new()
		.route("/capture", post(post_capture))
		.route("/capture", get(list_captures))
		.route("/capture/:session_id", get(get_capture))
		.route("/capture/:session_id", delete(delete_capture))
}

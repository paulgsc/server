use crate::handlers::outcomes as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
	routing::post,
	Router,
};

/// Where an applet reports how one block of a session went (#287, TEL2).
///
/// One route: the variety is in the body — see `handlers::outcomes` for what
/// it writes, what it derives, and what it refuses.
pub fn outcomes<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, AUTHORIZATION]);

	Router::new().route("/outcomes", post(handlers::record)).layer(cors)
}

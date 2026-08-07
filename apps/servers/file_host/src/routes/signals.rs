use crate::handlers::signals as handlers;
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

/// The ingress for domain events.
///
/// One route, because the interesting variety is in the body's type rather than
/// in the URL space: adding a signal is a variant in `study_domain`, not an
/// endpoint here.
pub fn signals<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, AUTHORIZATION]);

	Router::new().route("/signals", post(handlers::observe)).layer(cors)
}

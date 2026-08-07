use crate::handlers::github as routes;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::{AppState, Config};
use axum::routing::get;
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
	Router,
};

pub fn get_repos<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	// Was a hardcoded `http://nixos.local:6006` — Storybook's port, not the
	// app's. Now the same `ALLOWED_ORIGINS` list every other browser-facing
	// route uses, which is what lets the HTTPS study origin reach it.
	let cors = allowlisted_cors_with_credentials(config, vec![Method::GET], vec![CONTENT_TYPE, AUTHORIZATION]);

	Router::new()
		// TODO: Add path validation: something about must start with slashes?
		.route("/get_github_repos", get(routes::get_github_repos))
		.layer(cors)
}

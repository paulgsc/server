use crate::handlers::github as routes;
use crate::AppState;
use axum::routing::get;
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		{HeaderValue, Method},
	},
	Router,
};
use tower_http::cors::CorsLayer;

pub fn get_repos<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new()
		.allow_origin("http://nixos.local:6006".parse::<HeaderValue>().unwrap())
		.allow_methods([Method::GET])
		.allow_headers([CONTENT_TYPE, AUTHORIZATION])
		.allow_credentials(true);

	Router::new()
		// TODO: Add path validation: something about must start with slashes?
		.route("/get_github_repos", get(routes::get_github_repos))
		.layer(cors)
}

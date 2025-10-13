use crate::handlers::read_sheets as routes;
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

pub fn get_sheets<S>() -> Router<S>
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
		.route("/get_attributions/:sheet_id", get(routes::get_attributions))
		.route("/get_video_chapters/:sheet_id", get(routes::get_video_chapters))
		.route("/get_gantt/:sheet_id", get(routes::get_gantt))
		.route("/get_nfl_tennis/:sheet_id", get(routes::get_nfl_tennis))
		.route("/get_nfl_roster/:sheet_id", get(routes::get_nfl_roster))
		.layer(cors)
}

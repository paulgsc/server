use crate::handlers::read_sheets as routes;
use crate::routes::cors::allowlisted_cors;
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

pub fn get_sheets<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::GET], vec![CONTENT_TYPE, AUTHORIZATION]);

	Router::new()
		// TODO: Add path validation: something about must start with slashes?
		.route("/get_attributions/:sheet_id", get(routes::get_attributions))
		.route("/get_video_chapters/:sheet_id", get(routes::get_video_chapters))
		.route("/get_gantt/:sheet_id", get(routes::get_gantt))
		.route("/get_nfl_tennis/:sheet_id", get(routes::get_nfl_tennis))
		.route("/get_nfl_roster/:sheet_id", get(routes::get_nfl_roster))
		.layer(cors)
}

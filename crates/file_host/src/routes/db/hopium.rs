use crate::handlers::db::hopium as routes;
use crate::AppState;
use axum::routing::{delete, get, patch, post};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		HeaderValue, Method,
	},
	Router,
};
use tower_http::cors::CorsLayer;

pub fn mood_events<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new()
		.allow_origin("http://nixos.local:6006".parse::<HeaderValue>().unwrap())
		.allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
		.allow_headers([CONTENT_TYPE, AUTHORIZATION])
		.allow_credentials(true);

	Router::new()
		// Single mood event operations
		.route("/mood_events", post(routes::create_mood_event))
		.route("/mood_events", get(routes::get_all_mood_events))
		.route("/mood_events/:id", get(routes::get_mood_event_by_id))
		.route("/mood_events/:id", patch(routes::update_mood_event))
		.route("/mood_events/:id", delete(routes::delete_mood_event))
		// Batch operations
		.route("/mood_events/batch", post(routes::batch_create_mood_events))
		.route("/mood_events/batch", patch(routes::batch_update_mood_events))
		.route("/mood_events/batch", delete(routes::batch_delete_mood_events))
		// Query operations
		.route("/mood_events/week/:week", get(routes::get_mood_events_by_week))
		.route("/mood_events/team/:team", get(routes::get_mood_events_by_team))
		.route("/mood_events/stats", get(routes::get_mood_stats))
		.layer(cors)
}

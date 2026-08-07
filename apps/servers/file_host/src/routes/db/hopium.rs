use crate::handlers::db::hopium as routes;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::{AppState, Config};
use axum::routing::{delete, get, patch, post};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
	Router,
};

pub fn mood_events<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	// Was a hardcoded `http://nixos.local:6006` — Storybook's port, not the
	// app's. Now the same `ALLOWED_ORIGINS` list every other browser-facing
	// route uses, which is what lets the HTTPS study origin reach it.
	let cors = allowlisted_cors_with_credentials(config, vec![Method::GET, Method::POST, Method::PATCH, Method::DELETE], vec![CONTENT_TYPE, AUTHORIZATION]);

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
